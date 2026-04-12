# Arcadestr — Test Plan

**Version:** 1.0  
**Date:** 2026-04-09  
**Scope:** `arcadestr-core` crate (native feature flag); Tauri command layer; key NOSTR protocol flows  
**NIP references:** NIP-01, NIP-05, NIP-15, NIP-19, NIP-46, NIP-57, NIP-65, NIP-99

---

## 1. Guiding Principles

All tests in this plan are grounded in two sources of truth:

1. **The Nostr Implementation Possibilities (NIPs)** — the authoritative specification for how events must be structured, how keys must be encoded, how signatures must be verified, and how each protocol flow must behave.
2. **The Arcadestr codebase contracts** — the Rust types, Tauri commands, and storage layer described in `CODEBASE.md`, which must implement those NIPs correctly.

A test passes only when the code produces output that is **both internally consistent** and **compliant with the referenced NIP specification**.

---

## 2. Test Environment & Setup

```bash
# Run all core tests (single-threaded required for SQLite)
cargo test -p arcadestr-core --features native -- --test-threads=1

# Run a specific module
cargo test -p arcadestr-core --features native -- storage::

# Lint before committing
cargo clippy -p arcadestr-core --features native -- -D warnings
```

Place all new test modules in `core/src/<module>/tests.rs` (inline) or `core/tests/<module>_tests.rs` (integration). Use `#[cfg(test)]` for inline blocks and `#[tokio::test]` for async cases.

---

## 3. Module Test Suites

---

### 3.1 NIP-01 — Event Serialization & Validation

**NIP Reference:** NIP-01 defines the canonical event structure:

```
{ id, pubkey, created_at, kind, tags, content, sig }
```

The `id` MUST be the SHA-256 of the canonical JSON serialization of
`[0, pubkey, created_at, kind, tags, content]`. The `sig` MUST be a valid
Schnorr signature over `secp256k1`. All hex values (id, pubkey, sig) MUST be
64-character lowercase hex strings.

**File:** `core/src/nostr.rs`

| Test ID  | Name                                    | What to assert                                                                                                                    |
| -------- | --------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------- |
| NIP01-01 | `event_id_is_sha256_of_canonical_json`  | Serialize a test event, manually compute SHA-256 of `[0,pubkey,created_at,kind,tags,content]`, assert `event.id == computed_hash` |
| NIP01-02 | `event_id_is_32_byte_lowercase_hex`     | Assert `id.len() == 64` and all chars are `[0-9a-f]`                                                                              |
| NIP01-03 | `pubkey_is_32_byte_lowercase_hex`       | Same check on `pubkey` field                                                                                                      |
| NIP01-04 | `signature_is_64_byte_lowercase_hex`    | Assert `sig.len() == 128` and all chars are `[0-9a-f]`                                                                            |
| NIP01-05 | `schnorr_signature_verifies`            | Use `nostr` crate's verify function; reject any event where sig does not match                                                    |
| NIP01-06 | `tampered_event_fails_verification`     | Flip one bit in content, re-verify; expect `Err`                                                                                  |
| NIP01-07 | `kind_is_non_negative_integer`          | Assert `kind >= 0 && kind <= 65535`                                                                                               |
| NIP01-08 | `created_at_is_unix_timestamp`          | Assert `created_at > 0`, within ±5 years of now                                                                                   |
| NIP01-09 | `tags_are_arrays_of_strings`            | Each tag must be `Vec<String>` with at least one element                                                                          |
| NIP01-10 | `filter_since_until_ordering`           | Build a filter with `since=T`, `until=T+100`; verify only events where `since <= created_at <= until` match                       |
| NIP01-11 | `kind_0_metadata_content_is_valid_json` | Parse `kind:0` event content as JSON; assert it has at least a `name` key                                                         |

```rust
// Example scaffold
#[test]
fn event_id_is_sha256_of_canonical_json() {
    use sha2::{Sha256, Digest};
    use serde_json::json;

    let pubkey = "deadbeef..."; // 32-byte hex
    let created_at: u64 = 1700000000;
    let kind: u16 = 1;
    let tags: Vec<Vec<String>> = vec![];
    let content = "hello world";

    let canonical = json!([0, pubkey, created_at, kind, tags, content]).to_string();
    let expected_id = hex::encode(Sha256::digest(canonical.as_bytes()));

    let event = build_test_event(pubkey, created_at, kind, tags, content);
    assert_eq!(event.id, expected_id);
}
```

---

### 3.2 NIP-19 — Key Encoding (npub / nsec / bech32)

**NIP Reference:** NIP-19 specifies bech32 encoding with distinct prefixes to prevent confusion between key types. `npub` encodes a 32-byte public key; `nsec` encodes a 32-byte private key. NIP-19 keys MUST NOT be used inside NIP-01 event fields — only hex is valid there.

**File:** `core/src/nostr.rs`, `core/src/auth/`

| Test ID  | Name                                      | What to assert                                                                 |
| -------- | ----------------------------------------- | ------------------------------------------------------------------------------ |
| NIP19-01 | `npub_starts_with_npub1`                  | Any npub-encoded key starts with `"npub1"`                                     |
| NIP19-02 | `nsec_starts_with_nsec1`                  | Any nsec-encoded key starts with `"nsec1"`                                     |
| NIP19-03 | `npub_decode_roundtrip`                   | `encode(decode(npub)) == npub`                                                 |
| NIP19-04 | `hex_pubkey_and_npub_represent_same_key`  | Decode npub → hex → matches original pubkey hex                                |
| NIP19-05 | `npub_not_accepted_in_event_pubkey_field` | Attempting to create an event with `pubkey` set to an npub string should error |
| NIP19-06 | `unknown_bech32_prefix_is_ignored`        | Per NIP-19, unknown TLV types must not cause errors                            |
| NIP19-07 | `nprofile_includes_relay_hints`           | An `nprofile` entity includes at least one relay URL in its TLV data           |

---

### 3.3 NIP-05 — Identity Verification

**NIP Reference:** NIP-05 identifies a user by fetching
`https://<domain>/.well-known/nostr.json?name=<local-part>` and matching
the returned hex pubkey against the event's `pubkey`. Redirects MUST be ignored.
The identifier `_@domain.com` is valid and refers to the root of the domain.

**File:** `core/src/nip05_validator.rs`

| Test ID  | Name                                     | What to assert                                                                                             |
| -------- | ---------------------------------------- | ---------------------------------------------------------------------------------------------------------- |
| NIP05-01 | `valid_identifier_format`                | `user@domain.com` and `_@domain.com` are accepted; bare `username` is rejected                             |
| NIP05-02 | `hex_pubkey_match_passes_verification`   | Mock HTTP response with matching hex pubkey → `ValidationResult::Valid`                                    |
| NIP05-03 | `hex_pubkey_mismatch_fails_verification` | Mock response with wrong pubkey → `ValidationResult::Invalid`                                              |
| NIP05-04 | `http_redirect_is_not_followed`          | Mock server returns 301; validator must not follow → `ValidationResult::Error`                             |
| NIP05-05 | `missing_names_key_fails`                | Response JSON without `"names"` object → error                                                             |
| NIP05-06 | `npub_in_json_response_is_rejected`      | Per NIP-05: only hex pubkeys are valid in `.well-known/nostr.json` responses; npub format must be rejected |
| NIP05-07 | `cache_returns_cached_result`            | Call `validate` twice; second call must not make HTTP request (check with mock)                            |
| NIP05-08 | `relays_from_nostr_json_are_extracted`   | When the JSON includes a `"relays"` key, those relay URLs should be returned for relay discovery           |

---

### 3.4 NIP-15 — Marketplace Products (kind:30018)

**NIP Reference:** NIP-15 uses `kind:30018` for products and `kind:30017` for stalls. Products are parameterized replaceable events using a `d` tag as the unique identifier. The product content is a JSON object with fields including `id`, `name`, `description`, `images`, `price` (object with `amount`, `currency`, `frequency`).

**File:** `core/src/nostr.rs`, `core/src/marketplace_cache.rs`

| Test ID  | Name                                      | What to assert                                                                        |
| -------- | ----------------------------------------- | ------------------------------------------------------------------------------------- |
| NIP15-01 | `product_event_uses_kind_30018`           | Published NIP-15 `GameListing` with `source: Nip15Product` must produce `kind: 30018` |
| NIP15-02 | `product_has_d_tag`                       | The event's tags contain a `["d", "<slug>"]` entry                                    |
| NIP15-03 | `product_d_tag_is_unique_identifier`      | Two listings with different `id` fields produce distinct `d` tags                     |
| NIP15-04 | `stall_id_is_present_in_tags`             | Product event tags contain a stall reference (`["a", "30017:<pubkey>:<stall-id>"]`)   |
| NIP15-05 | `price_tag_format`                        | Tags include `["price", "<amount>", "<currency>"]`                                    |
| NIP15-06 | `t_tags_carry_game_categories`            | `tags: Vec<String>` fields are mapped to `["t", "<tag>"]` entries in the event        |
| NIP15-07 | `image_tags_present_when_images_provided` | Each URL in `images: Vec<String>` produces an `["image", "<url>"]` tag                |
| NIP15-08 | `free_product_has_zero_price`             | A listing with `price: 0.0` serializes with price `0`                                 |

---

### 3.5 NIP-99 — Classified Listings (kind:30402)

**NIP Reference:** NIP-99 uses `kind:30402` for active listings and `kind:30403` for drafts. Content is Markdown. Required tags: `d`, `title`. Optional tags: `price`, `location`, `image`, `summary`, `published_at`, `t`, `e`, `a`.

**File:** `core/src/nostr.rs`

| Test ID  | Name                                 | What to assert                                                   |
| -------- | ------------------------------------ | ---------------------------------------------------------------- |
| NIP99-01 | `listing_event_uses_kind_30402`      | `GameListing` with `source: Nip99Listing` produces `kind: 30402` |
| NIP99-02 | `draft_listing_uses_kind_30403`      | A draft/inactive listing uses `kind: 30403`                      |
| NIP99-03 | `content_is_markdown_string`         | The event `content` field is a non-empty string (Markdown)       |
| NIP99-04 | `title_tag_is_present`               | Tags contain `["title", "<game-title>"]`                         |
| NIP99-05 | `published_at_tag_is_unix_timestamp` | `["published_at", "<ts>"]` where `<ts>` parses as a valid u64    |
| NIP99-06 | `price_tag_currency_codes`           | `["price", "100", "SATS"]` — currency is uppercase code          |
| NIP99-07 | `summary_tag_when_summary_present`   | If `summary: Some(s)` is set, `["summary", s]` appears in tags   |
| NIP99-08 | `location_tag_when_location_present` | If `location: Some(l)` is set, `["location", l]` appears in tags |

---

### 3.6 NIP-46 — Remote Signer (NostrConnect)

**NIP Reference:** NIP-46 defines two URI schemes:

- **bunker://** (user-initiated): `bunker://<remote-signer-pubkey>?relay=<wss://...>&secret=<optional>`
- **nostrconnect://** (client-initiated QR): `nostrconnect://<client-pubkey>?relay=<wss://...>&metadata=<json>&secret=<required>`

Communication uses `kind:24133` events with NIP-04 encrypted JSON-RPC payloads.
After `connect`, client MUST call `get_public_key` to discover the user pubkey.
The client-generated keypair is ephemeral and distinct from the user's keypair.

**File:** `core/src/nip46/`, `core/src/signers/nip46.rs`, `desktop/src/nip46_commands.rs`

| Test ID  | Name                                      | What to assert                                                                                                                           |
| -------- | ----------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------- |
| NIP46-01 | `bunker_uri_parsing`                      | `bunker://abc123?relay=wss://relay.example.com&secret=xyz` → remote pubkey = `abc123`, relay = `wss://relay.example.com`, secret = `xyz` |
| NIP46-02 | `bunker_uri_multiple_relays`              | Parse `?relay=wss://a.com&relay=wss://b.com` → two relay URLs                                                                            |
| NIP46-03 | `bunker_uri_without_secret`               | Missing `secret` param → `None` (not an error)                                                                                           |
| NIP46-04 | `nostrconnect_uri_format`                 | `generate_nostrconnect_uri` output starts with `nostrconnect://`, contains `relay=` and `secret=`                                        |
| NIP46-05 | `nostrconnect_client_pubkey_is_ephemeral` | The pubkey in a generated `nostrconnect://` URI is NOT the user's pubkey                                                                 |
| NIP46-06 | `request_event_uses_kind_24133`           | An outgoing NIP-46 request event has `kind: 24133`                                                                                       |
| NIP46-07 | `request_payload_is_nip04_encrypted`      | The `content` field of a kind-24133 event is NIP-04 encrypted (ciphertext?iv format)                                                     |
| NIP46-08 | `request_payload_json_rpc_structure`      | Decrypted payload is `{id, method, params: [...]}`                                                                                       |
| NIP46-09 | `response_payload_json_rpc_structure`     | Decrypted response is `{id, result, error}`                                                                                              |
| NIP46-10 | `connect_method_returns_ack`              | `connect` response `result` equals the secret from the bunker URI                                                                        |
| NIP46-11 | `get_public_key_after_connect`            | After successful `connect`, `get_public_key` returns a valid 32-byte hex pubkey                                                          |
| NIP46-12 | `sign_event_returns_valid_signature`      | `sign_event` response contains an event with a valid Schnorr sig                                                                         |
| NIP46-13 | `connection_status_transitions`           | Status goes `Disconnected → Connecting → Connected`; never skips states                                                                  |
| NIP46-14 | `saved_profile_persists_across_restart`   | Save a profile, clear in-memory state, reload — profile reappears                                                                        |
| NIP46-15 | `wrong_secret_rejected`                   | If `connect` response secret ≠ expected secret → `Err`                                                                                   |
| NIP46-16 | `ping_returns_pong`                       | `ping` method must return `"pong"`                                                                                                       |

---

### 3.7 NIP-57 — Lightning Zaps

**NIP Reference:** NIP-57 zap flow:

1. Client fetches recipient's LNURL endpoint from `lud16` field.
2. If `allowsNostr: true` and `nostrPubkey` is valid BIP-340, client builds a `kind:9734` zap request event.
3. Zap request is sent to the LNURL callback URL as `?nostr=<uri-encoded-event>`.
4. If an `amount` tag is present in the zap request, it MUST match the amount query parameter.
5. Server responds with a BOLT11 invoice. Client pays it.
6. Server publishes a `kind:9735` zap receipt. Receipt MUST include `bolt11` and `description` tags.

**File:** `core/src/lightning.rs`

| Test ID  | Name                                            | What to assert                                                                               |
| -------- | ----------------------------------------------- | -------------------------------------------------------------------------------------------- |
| NIP57-01 | `zap_request_uses_kind_9734`                    | `ZapRequest` serializes to `kind: 9734`                                                      |
| NIP57-02 | `zap_request_has_relays_tag`                    | Tags include `["relays", "wss://relay1", "wss://relay2"]`                                    |
| NIP57-03 | `zap_request_amount_tag_matches_query_param`    | If `amount` tag is present, its value equals the `?amount=` in the LNURL callback URL        |
| NIP57-04 | `zap_request_p_tag_is_recipient_hex_pubkey`     | `["p", "<hex-pubkey>"]` (not npub)                                                           |
| NIP57-05 | `zap_request_e_tag_when_zapping_event`          | When zapping a specific event, `["e", "<event-id>"]` is present                              |
| NIP57-06 | `lud16_parsing`                                 | `user@domain.com` → LNURL endpoint `https://domain.com/.well-known/lnurlp/user`              |
| NIP57-07 | `lnurl_response_allows_nostr`                   | If `allowsNostr` is false or absent → do not build zap request, fall back to regular invoice |
| NIP57-08 | `zap_receipt_kind_9735`                         | Receipt event (parsed from relay) must be `kind: 9735`                                       |
| NIP57-09 | `zap_receipt_has_bolt11_tag`                    | Receipt tags include `["bolt11", "<invoice>"]`                                               |
| NIP57-10 | `zap_receipt_description_sha256_matches_bolt11` | `SHA256(description tag)` == description hash in BOLT11 invoice                              |
| NIP57-11 | `max_sendable_respected`                        | Amount request must not exceed `maxSendable` from LNURL response                             |

---

### 3.8 NIP-65 — Relay List Metadata & Relay Selection

**NIP Reference:** NIP-65 uses `kind:10002` to publish a user's relay preferences. Each relay entry has an `r` tag with optional `read` or `write` marker. The outbox model: events are published to a user's write relays; queries are made to their read relays.

**File:** `core/src/relay_cache.rs`, `core/src/relay_manager.rs`, `core/src/nostr.rs`

| Test ID  | Name                                          | What to assert                                                                                                                            |
| -------- | --------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------- |
| NIP65-01 | `relay_list_event_uses_kind_10002`            | Saved relay list serializes as `kind: 10002`                                                                                              |
| NIP65-02 | `r_tag_format`                                | Each relay in the list appears as `["r", "wss://relay.url"]` or `["r", "wss://relay.url", "read"]` or `["r", "wss://relay.url", "write"]` |
| NIP65-03 | `relay_without_marker_is_both_read_and_write` | An `r` tag with no marker is treated as both read and write                                                                               |
| NIP65-04 | `save_and_load_relay_list`                    | Save a `CachedRelayList`, reload from SQLite, assert equality                                                                             |
| NIP65-05 | `greedy_set_cover_selects_minimum_relays`     | Given 3 pubkeys each covered by relay A (all 3), B (2), and C (1) — greedy selection picks A first                                        |
| NIP65-06 | `relay_selection_covers_all_pubkeys`          | After `select_relays()`, `uncovered_pubkeys` is empty when sufficient relays exist                                                        |
| NIP65-07 | `relay_selection_respects_max_relays`         | With `max_relays=2`, at most 2 relays are returned even if more could improve coverage                                                    |
| NIP65-08 | `relay_status_transitions`                    | `RelayStatus { connected: false }` after disconnect; `connected: true` after reconnect                                                    |
| NIP65-09 | `latency_ms_is_none_before_first_ping`        | Fresh `RelayStatus` has `latency_ms: None`                                                                                                |

---

### 3.9 Storage Layer

**File:** `core/src/storage/`

#### 3.9.1 Database / Migrations

| Test ID | Name                                | What to assert                                                                   |
| ------- | ----------------------------------- | -------------------------------------------------------------------------------- |
| STG-01  | `database_initializes_successfully` | `Database::new(":memory:")` completes without error                              |
| STG-02  | `migrations_run_idempotently`       | Running migrations twice on the same database does not error or duplicate data   |
| STG-03  | `schema_version_increments`         | After each migration, `PRAGMA user_version` equals the expected migration number |

#### 3.9.2 MarketplaceCache

| Test ID | Name                                         | What to assert                                                                             |
| ------- | -------------------------------------------- | ------------------------------------------------------------------------------------------ |
| MKT-01  | `upsert_new_listing_returns_inserted`        | First upsert of a listing → `UpsertOutcome::Inserted`                                      |
| MKT-02  | `upsert_unchanged_listing_returns_unchanged` | Second upsert of same listing (no field changes) → `UpsertOutcome::Unchanged`              |
| MKT-03  | `upsert_changed_listing_returns_updated`     | Upsert with `title` changed → `UpsertOutcome::Updated`                                     |
| MKT-04  | `load_listings_returns_all_upserted`         | Upsert 5 listings, `load_listings()` returns all 5                                         |
| MKT-05  | `listing_identity_is_publisher_plus_d_tag`   | Two listings with the same `publisher_npub` but different `id` are stored as separate rows |
| MKT-06  | `listing_ordering_by_created_at_desc`        | `load_listings()` returns most recent event first                                          |

#### 3.9.3 UserCache

| Test ID | Name                                | What to assert                                                       |
| ------- | ----------------------------------- | -------------------------------------------------------------------- |
| USR-01  | `get_returns_none_for_unknown_npub` | `get("unknown_npub")` → `None`                                       |
| USR-02  | `put_and_get_roundtrip`             | `put(profile)` then `get(npub)` returns identical profile            |
| USR-03  | `put_overwrites_stale_profile`      | Put profile v1, then put profile v2 with same npub; `get` returns v2 |

#### 3.9.4 Encryption

| Test ID | Name                                        | What to assert                                                           |
| ------- | ------------------------------------------- | ------------------------------------------------------------------------ |
| ENC-01  | `encrypt_decrypt_roundtrip`                 | `decrypt(encrypt(plaintext, key), key) == plaintext`                     |
| ENC-02  | `different_keys_cannot_decrypt`             | Encrypting with key A, decrypting with key B → `Err`                     |
| ENC-03  | `ciphertext_differs_from_plaintext`         | `encrypt(x) != x`                                                        |
| ENC-04  | `nonce_is_unique_per_encryption`            | Encrypt the same plaintext twice; ciphertexts must differ (random nonce) |
| ENC-05  | `master_key_derivation_is_deterministic`    | Same password + salt → same master key                                   |
| ENC-06  | `master_key_derivation_different_passwords` | Different passwords → different keys (Argon2 test)                       |

---

### 3.10 Signer Abstraction

**File:** `core/src/signers/local.rs`

| Test ID | Name                                          | What to assert                                                                 |
| ------- | --------------------------------------------- | ------------------------------------------------------------------------------ |
| SGN-01  | `local_signer_public_key_matches_private_key` | `signer.public_key()` matches the pubkey derived from the provided private key |
| SGN-02  | `local_signer_sign_produces_valid_event`      | `sign_event(unsigned)` → event with valid Schnorr signature                    |
| SGN-03  | `unsigned_event_is_rejected`                  | Submitting an event to the relay without a valid sig must be rejected          |
| SGN-04  | `sign_event_sets_pubkey`                      | The signed event's `pubkey` field equals `signer.public_key()`                 |

---

### 3.11 Profile Fetcher

**File:** `core/src/profile_fetcher.rs`

| Test ID | Name                                  | What to assert                                                   |
| ------- | ------------------------------------- | ---------------------------------------------------------------- |
| PF-01   | `enqueue_does_not_add_already_cached` | If npub is in cache, `enqueue_many` does not add it to the queue |
| PF-02   | `enqueue_does_not_add_duplicates`     | Calling `enqueue_many` with same npub twice adds it only once    |
| PF-03   | `batch_size_capped_at_10`             | `fetch_batch` processes at most 10 items per call                |
| PF-04   | `processed_items_removed_from_queue`  | After `fetch_batch`, fetched npubs are gone from the queue       |

---

### 3.12 Relay Hints (NIP-19 TLV)

**File:** `core/src/relay_hints.rs`

| Test ID | Name                              | What to assert                                                         |
| ------- | --------------------------------- | ---------------------------------------------------------------------- |
| RH-01   | `p_tag_with_relay_hint_is_stored` | An event with `["p", "<pubkey>", "<relay-url>"]` stores the relay hint |
| RH-02   | `relay_hints_retrieved_by_pubkey` | `get_hints(pubkey)` returns the relays seen in p-tags for that pubkey  |
| RH-03   | `relay_hints_deduplicated`        | Inserting the same relay hint twice stores it once                     |

---

## 4. Integration Test Scenarios

These tests exercise multiple modules together and are placed in `core/tests/`.

### INT-01: Full Listing Publish & Retrieve Cycle

```
Given: An authenticated LocalSigner
When: publish_listing(game_listing) is called
Then:
  - The returned event ID is a valid 32-byte hex string
  - The event kind matches the ListingSource (30018 or 30402)
  - The event has a valid Schnorr signature
  - fetch_listing_by_id(publisher_npub, listing_id) returns the same listing
  - The listing is stored in MarketplaceCache
```

### INT-02: NIP-05 Validation with Relay Discovery

```
Given: A UserProfile with nip05 = "alice@example.com"
When: Nip05Validator::validate("alice@example.com", alice_pubkey_hex) is called
  AND the HTTP response includes a "relays" map
Then:
  - nip05_verified = true is set on the profile
  - The relay URLs from the response are available via relay discovery
  - A second call within cache TTL does not make an HTTP request
```

### INT-03: NIP-46 Session Lifecycle

```
Given: A mocked bunker relay
When:
  1. start_qr_login() is called → get nostrconnect:// URI
  2. The mock bunker "scans" the URI and sends a kind:24133 connect response
  3. check_qr_connection() is polled
Then:
  - Connection status transitions: Connecting → Connected
  - get_public_key() returns the user's pubkey (not the ephemeral client key)
  - list_saved_profiles() includes the newly connected profile
  - After logout_nip46(), is_authenticated() returns false
```

### INT-04: Marketplace Cache Streaming

```
Given: A relay emitting 20 kind:30402 events
When: fetch_marketplace_stream(limit=20, since_days=None) is called
Then:
  - Each received listing emits a "marketplace-product" Tauri event
  - "marketplace-complete" event fires after all listings are processed
  - load_listings() on MarketplaceCache returns up to 20 entries
  - Duplicate event IDs are not stored twice (deduplication)
  - Cached listings from a previous session are served first (fast initial render)
```

### INT-05: NIP-57 Zap Invoice Request

```
Given: A GameListing with lud16 = "seller@walletofsatoshi.com"
  AND: A mocked LNURL endpoint returning { allowsNostr: true, callback: "...", ... }
When: request_zap_invoice(zap_request) is called
Then:
  - A kind:9734 event is created with the correct p, relays, and amount tags
  - The event is sent to the LNURL callback URL as URI-encoded ?nostr=<event>
  - A valid BOLT11 invoice string is returned in ZapInvoice
  - If allowsNostr is false, no kind:9734 event is created
```

---

## 5. Error & Edge Case Tests

| Test ID | Scenario                                        | Expected Behavior                                                                  |
| ------- | ----------------------------------------------- | ---------------------------------------------------------------------------------- |
| ERR-01  | Relay returns HTML instead of WebSocket         | `RelayError` returned; relay marked as disconnected                                |
| ERR-02  | NIP-46 connect timeout                          | After configurable timeout, state → `ConnectionFailed`; error surfaced to frontend |
| ERR-03  | Malformed `bunker://` URI (missing relay param) | `Nip46Error::InvalidUri`                                                           |
| ERR-04  | `publish_listing` when `NotAuthenticated`       | Returns `Err("Not authenticated")`                                                 |
| ERR-05  | `fetch_profile` with invalid npub               | Returns `Err` with descriptive message; does not panic                             |
| ERR-06  | SQLite write fails (disk full simulation)       | Returns `Err`; does not corrupt existing data                                      |
| ERR-07  | `lud16` contains no `@` symbol                  | `lightning::parse_lud16` returns `Err`                                             |
| ERR-08  | LNURL endpoint unreachable                      | Returns `Err`; timeout respected                                                   |
| ERR-09  | `amount` in zap request exceeds `maxSendable`   | Returns `Err` before making HTTP call                                              |
| ERR-10  | Kind-0 event with invalid JSON content          | `fetch_profile` returns `Err("Malformed event")`                                   |

---

## 6. Tauri Command Layer Tests

These test the serialization contracts between frontend and backend. Run with `cargo test -p arcadestr-desktop`.

| Command                | Test                       | Assertion                                                           |
| ---------------------- | -------------------------- | ------------------------------------------------------------------- |
| `is_authenticated`     | Before any login           | Returns `false`                                                     |
| `get_public_key`       | Before login               | Returns `Err`                                                       |
| `connect_with_key`     | Valid nsec input           | `is_authenticated()` → `true`; emits `auth_success` event           |
| `get_version_info`     | Always                     | Returns `VersionInfo` with non-empty `version` string               |
| `get_connected_relays` | After startup              | Returns `Vec<RelayStatus>` (may be empty)                           |
| `publish_listing`      | Valid `GameListing` struct | Returns event ID string; `kind` in response matches `ListingSource` |
| `fetch_profile`        | Valid npub                 | Returns `UserProfile` with matching `npub` field                    |
| `list_saved_profiles`  | After adding profile       | Result contains the added profile                                   |
| `delete_profile`       | Known profile_id           | Subsequent `list_saved_profiles` does not include it                |

**Serialization contracts to verify:**

- All commands that return `serde_json::Value` must deserialize cleanly into the expected Rust struct.
- Commands returning `Vec<T>` return an empty array `[]` (not `null`) when there are no results.
- Error responses are strings (not JSON objects) as required by the Tauri command pattern.

---

## 7. Test Utilities & Helpers

Create `core/src/test_helpers.rs` (behind `#[cfg(test)]`):

```rust
/// Build a minimal valid unsigned NOSTR event for testing
pub fn test_unsigned_event(kind: u64, content: &str) -> UnsignedEvent {
    UnsignedEvent {
        created_at: Timestamp::now(),
        kind: Kind::from(kind),
        tags: Tags::default(),
        content: content.to_string(),
    }
}

/// Build a minimal GameListing for testing
pub fn test_game_listing(source: ListingSource) -> GameListing {
    GameListing {
        id: "test-game-v1".to_string(),
        source,
        title: "Test Game".to_string(),
        description: "A test game".to_string(),
        price: 1000.0,
        currency: "SATS".to_string(),
        price_sats: 1000,
        lud16: "test@walletofsatoshi.com".to_string(),
        publisher_npub: "npub1...".to_string(),
        stall_id: "stall-001".to_string(),
        created_at: 1700000000,
        ..Default::default()
    }
}

/// In-memory SQLite database for isolated storage tests
pub async fn test_database() -> Database {
    Database::new(":memory:").await.expect("in-memory db failed")
}
```

---

## 8. Coverage Targets

| Module                               | Target Line Coverage | Priority            |
| ------------------------------------ | -------------------- | ------------------- |
| `core/src/storage/encryption.rs`     | 90%                  | Critical (crypto)   |
| `core/src/nip46/`                    | 85%                  | Critical (auth)     |
| `core/src/nostr.rs` (event building) | 85%                  | Critical (protocol) |
| `core/src/marketplace_cache.rs`      | 80%                  | High                |
| `core/src/nip05_validator.rs`        | 80%                  | High                |
| `core/src/lightning.rs`              | 75%                  | High                |
| `core/src/relay_cache.rs`            | 75%                  | Medium              |
| `core/src/profile_fetcher.rs`        | 70%                  | Medium              |

Run coverage with:

```bash
cargo llvm-cov --features native -p arcadestr-core --html
```

---

## 9. Known Constraints

- **SQLite tests must run with `--test-threads=1`** (shared file descriptor on SQLite).
- **NIP-46 and relay tests require mocking** — do not connect to real relays in unit tests. Use a `MockRelay` or `MockHttpClient` pattern.
- **WASM target is out of scope** for this plan; `core` tests use the `native` feature flag only.
- **Tauri command tests** require the `AppState` to be constructed explicitly without calling `tauri::Builder`; extract command logic into testable free functions where possible.
