# Arcadestr Blossom Media Upload Implementation Plan

## 1. Recommended Blossom BUD Scope

All current Blossom BUDs are drafts. Implement their normative requirements without presenting them as finalized standards.

- **BUD-00/01:** Follow baseline server and hash-addressed blob retrieval conventions.
- **BUD-02:** Upload with `PUT /upload` and a streamed binary body. Send `Content-Type`, `Content-Length`, and lowercase `X-SHA-256`. Accept only `200 OK` for an existing blob or `201 Created` for a newly stored blob.
- **BUD-06:** Optionally perform a per-file `HEAD /upload` preflight with `X-SHA-256`, `X-Content-Type`, and `X-Content-Length`. Treat `404`, `405`, or `501` as unsupported preflight and continue with `PUT`; preflight is not general capability discovery and does not guarantee upload success.
- **BUD-11:** Sign a kind `24242` authorization event with human-readable content, `t=upload`, a future NIP-40 `expiration`, required lowercase `x=<sha256>`, and a recommended lowercase-domain `server` tag. Send it as `Authorization: Nostr <base64url-without-padding event JSON>`.
- **Blob descriptor:** Require `url`, `sha256`, `size`, `type`, and `uploaded`; ignore unknown additional fields. Verify relevant fields before accepting the upload.
- **Authorization size binding:** BUD-11 binds the operation and blob hash, but not size. Size remains an HTTP `Content-Length` or BUD-06 `X-Content-Length` value. Do not invent an authorization tag for it.
- **Capability discovery:** Blossom currently defines no general server-capability document or endpoint. Report servers as unchecked until a file-specific preflight or upload provides a concrete result.
- **Redirects:** Reject redirects for upload and preflight. Blossom specifies redirects for blob retrieval, not upload.
- **Deferred BUDs:** Defer BUD-03 server-list discovery, BUD-04 mirroring, BUD-05 optimization/transcoding, BUD-07 payment handling, and BUD-12 deletion/listing. Report `402 Payment Required` distinctly rather than as a generic failure.

BUD-03 discovery is intentionally deferred until Arcadestr supports mirroring or fallback behavior. The first implementation uses only publisher-configured servers and the server explicitly selected for the current upload. It must not automatically contact the first server from a kind `10063` event.

## 2. Key Architecture Decisions

### Core

- Add a reusable Blossom protocol module, likely `core/src/blossom.rs`, exported by `core/src/lib.rs`.
- Define authorization constants and types, unsigned authorization-event construction and parsing, HTTP authorization encoding, blob descriptors, descriptor validation, server URL validation, MIME and size policy, and integrity metadata validation.
- Keep signing and signer secrets outside the Blossom protocol module. Core constructs an unsigned event; the desktop signer abstraction signs it.
- Extend `StorePageMediaItem` in `core/src/store_page.rs` with backward-compatible optional integrity metadata.
- Keep Store Page event validation and sanitization authoritative in the existing Store Page protocol and content-policy modules.
- Do not add active BUD-03 relay discovery in the first implementation. Server-list types or parsing should be added only when the later mirroring/fallback phase is designed.

### Desktop

- Add a reusable native upload service, likely `desktop/src/blossom_upload.rs`, rather than embedding upload logic in `store_page_commands.rs`.
- Keep native file paths and file bytes in the backend. The picker returns an opaque selection ID and safe metadata, never a path.
- Inspect file content, enforce policy, stream SHA-256 hashing, stream the HTTP request body, bound response bodies, verify descriptors, and manage retry and cancellation in the backend.
- Add Tauri commands in a focused module such as `desktop/src/blossom_commands.rs` and register them in `desktop/src/main.rs`.
- Reuse the existing local-key and NIP-46 signer abstraction. Extract directly reusable active-signer/account-check helpers into a neutral desktop helper rather than duplicating them or coupling Blossom to ADP commands.
- Maintain cancellation tokens keyed by upload ID. A separate command cancels an active operation.
- Include an upload ID in every progress event so concurrent or stale operations cannot consume each other's events.
- Recheck the expected publisher before signing, after signing, before upload, and before returning success.
- Keep server configuration credential-free and account-specific in the existing desktop settings storage.

### App

- Add typed invoke/listener wrappers and explicit standalone-web fallbacks in `app/src/tauri_bridge.rs`.
- Implement a focused upload dialog/component and integrate it into the existing media section in `app/src/ui_v2/views/store_page_publish.rs`.
- Preserve the existing operation-generation and account-response checks.
- Keep manual URL entry as an unchanged first-class workflow.
- Do not redesign unrelated Publisher Studio sections.

### Preview Scope

- Do not implement a custom opaque local video resource or Range-serving handler unless testing proves it necessary.
- A small, validated image may use an existing safe file-preview mechanism if it does not expose unrestricted paths or move large bytes through IPC.
- If no existing mechanism satisfies those constraints, omit pre-upload image preview initially rather than broadening filesystem access.
- Video preview initially uses only the verified returned HTTPS URL after upload.
- Paths must never be returned to the WebView, and large files must never be base64-encoded or buffered in WebView memory.

## 3. Store Page Schema Recommendation

Extend `StorePageMediaItem` with three optional fields:

```json
{
  "sha256": "<lowercase-hex>",
  "mime_type": "image/webp",
  "size": 123456
}
```

Use optional Rust fields with absent values omitted during serialization:

- `sha256: Option<String>`
- `mime_type: Option<String>`
- `size: Option<u64>`

Rules:

- Existing Store Page events and manually entered HTTPS URLs remain valid with all three fields absent.
- A successful Blossom upload automatically supplies all three values from backend-verified data.
- If any integrity field is present, require the complete trio.
- Require exactly 64 lowercase hexadecimal characters for `sha256`.
- Require a supported MIME type consistent with the Store Page `type` value (`image` or `video`).
- Require a positive size within the configured policy.
- Preserve the fields through parsing and sanitization when valid.
- Treat integrity metadata in received Store Page events as publisher assertions; only the local upload flow can state that it verified the selected bytes.
- Keep one canonical `url`; Blossom upload does not require multiple URLs in the descriptor.
- Future mirroring can derive candidates from the hash and a server list or add an optional `mirrors` field without changing the canonical URL or blob-level integrity fields.

## 4. Supported Files and Limits

The initial implementation supports:

| Format | Detected MIME | Maximum size |
|---|---|---:|
| JPEG | `image/jpeg` | 20 MiB |
| PNG | `image/png` | 20 MiB |
| WebP | `image/webp` | 20 MiB |
| MP4 | `video/mp4` | 500 MiB |
| WebM | `video/webm` | 500 MiB |

GIF is deferred. Animated GIFs are inefficient, can create excessive decoder memory use, and overlap with MP4/WebM. Static-GIF detection is not included in the first implementation.

Validation rules:

- Never trust file extensions or picker-provided MIME values.
- Inspect content signatures before hashing authorization or upload.
- Validate ISO-BMFF brands sufficiently to distinguish MP4 from unrelated containers such as HEIC.
- Validate the EBML document type as WebM rather than accepting arbitrary Matroska/EBML files.
- Reject ambiguous, truncated, or unsupported containers before upload.
- Recheck actual file size while opening and reading to catch replacement or mutation after selection.
- Align videos with the existing Store Page policy: MP4 and WebM only.
- Do not promise codec compatibility or transcode media in the first implementation.
- Never read a complete video into memory.

## 5. Server Configuration and Discovery

The first implementation uses this deterministic source order:

1. Server explicitly selected for the current upload.
2. Publisher-account-configured Blossom servers.
3. No fallback.

There is no Arcadestr-controlled default server, and no kind `10063` discovery in the first implementation.

Configuration rules:

- Store non-secret server origins in desktop settings keyed by publisher pubkey.
- Do not store authorization events; generate short-lived authorization for each attempt.
- Require an origin URL with no username, password, query, fragment, or non-root path.
- Require HTTPS in production.
- Permit HTTP only in explicit development mode for exact loopback hosts: `localhost`, `127.0.0.1`, or `[::1]`.
- Reject private, link-local, multicast, unspecified, and metadata-service destinations.
- Normalize and deduplicate origins while preserving user order.
- Clear the current per-upload selection when the account changes and load only the new account's configured servers.
- Show newly configured servers as unchecked. Do not infer compatibility from a generic request.
- Perform file-specific BUD-06 checks only after the file has been validated and hashed.
- Display concrete states such as preflight accepted, preflight unsupported, unreachable, authorization rejected, payment required, media rejected, or size rejected.

## 6. Upload and Security Flow

The complete flow is:

```text
select file
-> inspect metadata and content
-> validate MIME and size
-> compute SHA-256
-> choose and validate Blossom server
-> create signed authorization
-> optionally preflight
-> stream upload
-> verify response hash, size, type, and URL
-> insert media item into the Store Page draft
```

Detailed behavior:

1. The native picker creates an account-bound opaque selection record containing the backend path and immutable observed metadata.
2. The backend reopens the file, sniffs its content, reads metadata, and enforces type and size policy.
3. Hashing streams through the existing chunked SHA-256 pattern and reports cumulative progress.
4. The selected server origin is validated against scheme, credential, host, path, and network-destination rules.
5. Arcadestr builds a human-readable kind `24242` event scoped to `upload`, the computed hash, expiration, and server domain.
6. The shared active signer signs the event. The UI displays a waiting state for NIP-46 and handles rejection or delay without losing the selection.
7. Arcadestr may send an authenticated BUD-06 preflight. Unsupported preflight falls through to upload.
8. Arcadestr streams the exact selected file through `PUT /upload` with `Content-Type`, `Content-Length`, `X-SHA-256`, and authorization headers.
9. The backend accepts only `200` or `201`, reads at most 64 KiB of response data, parses the descriptor, and verifies every required field.
10. After a final account check, the app inserts a structured media item with canonical URL and verified integrity metadata.

Progress phases should include `inspect`, `hash`, `sign`, `preflight`, `upload`, and `verify`, with cumulative bytes and total bytes where applicable.

Cancellation and retry rules:

- Cancellation stops local reading and drops the request body as soon as possible.
- The server may retain a partial or completed blob; communicate that possibility.
- A retry reuses the still-valid opaque selection but reopens and revalidates the file.
- Recompute the hash if observed file metadata changed.
- Generate a fresh authorization if the prior event is expired or close to expiration.
- Retry the complete hash-addressed PUT. Use explicit user retry after any body bytes were sent.
- Treat duplicate `200` responses as success only after full descriptor verification.
- If the account changes, cancel when possible and never apply the response to the draft.
- If upload succeeded remotely but response verification or account checks fail, report a possible orphaned blob and do not insert it.
- Removing an uploaded item from the draft does not delete its remote blob. Blob deletion remains deferred.

Security rules:

- Use HTTPS-only production servers and narrow loopback-only development exceptions.
- Disable redirects for both preflight and upload.
- Resolve and validate destinations to mitigate SSRF and DNS rebinding; reject any non-public destination outside explicit loopback development mode.
- Bound error text, `X-Reason`, headers, and JSON response size.
- Use a short authorization lifetime; ten minutes is the initial recommendation to accommodate NIP-46 approval.
- Reject hash, size, or MIME mismatches even when the server claims success.
- Require credential-free descriptor URLs accepted by the existing Store Page HTTPS policy.
- Verify that the descriptor URL's last 64-character hexadecimal hash occurrence matches the selected file hash.
- Do not automatically fetch the returned descriptor URL during verification.
- Treat all response fields and diagnostics as untrusted data.
- Keep signer secrets inside existing signer implementations; Blossom receives only the signed authorization event.
- Keep paths and file bytes outside Tauri IPC payloads.
- Do not grant broad Tauri filesystem, shell, or HTTP capabilities for this feature.
- CSP is defense in depth, not the upload security boundary.

## 7. UI Workflow

Keep the Store Page media workflow focused:

```text
[Upload from computer] [Use existing URL]
```

The upload dialog shows:

- selected filename without its local path;
- detected MIME type and size;
- selected configured Blossom server;
- safe local image preview when an existing narrow mechanism is available;
- hashing, signer, preflight, upload, and verification states;
- byte progress during hashing and upload;
- cancel and retry controls;
- verified resulting URL and SHA-256;
- actionable protocol and policy errors.

After insertion, the existing structured media editor remains responsible for role, caption, alt text, ordering, and deletion from the draft. Video preview uses the verified HTTPS URL after upload. Manual URL entry remains unchanged and does not require integrity metadata.

## 8. Focused Testing

### Core tests

- Correct kind `24242` authorization generation and parsing.
- Required human-readable content, `t`, `expiration`, `x`, and server tags.
- Base64url-without-padding authorization header encoding.
- Rejection of malformed, duplicate, mismatched, expired, or incorrectly scoped authorization data.
- Blob descriptor parsing and required-field validation.
- Lowercase SHA-256 validation.
- Valid complete Store Page integrity metadata.
- Rejection of partial integrity metadata.
- Backward-compatible parsing of media without integrity metadata.
- Existing manual URL workflow remains valid.

### Desktop tests

- Local-key signing through the shared signer abstraction.
- NIP-46 signing, delay, rejection, and timeout behavior with existing mocks.
- Known-file SHA-256 verification.
- JPEG, PNG, WebP, MP4, and WebM content detection.
- GIF and other unsupported-file rejection.
- Image and video size rejection.
- Valid `201` upload response.
- Valid duplicate-existing `200` response.
- Hash, size, MIME, and descriptor URL mismatch.
- Unsafe server and returned URLs.
- Redirect rejection for preflight and upload.
- Maximum response size enforcement.
- Cancellation before and during body streaming.
- Retry after transient connection failure and partial send.
- Stale-account response rejection.
- File replacement or mutation after selection.
- Distinct handling for `401`, `402`, `413`, `415`, and `429`.

### App tests

- Desktop bridge request and event serialization.
- Explicit unsupported standalone-web behavior.
- Correlation of progress by upload ID.
- Successful media draft insertion with verified metadata.
- Stale operation-generation response ignored.
- Account switching clears active upload UI.
- Cancel and retry state transitions.
- Role, caption, and alt-text editing after upload.
- Removing an item does not request remote deletion.
- Manual URL workflow remains unchanged.

## 9. Phased Implementation

### Phase 1: Protocol Types and Store Page Schema

Scope:

- Add Blossom protocol constants, authorization types/builders/parsers, authorization header encoding, blob descriptor types and validation, server-origin validation, and MIME/size policy in core.
- Extend `StorePageMediaItem` with optional `sha256`, `mime_type`, and `size`.
- Update Store Page validation and sanitization for complete, valid integrity metadata.
- Add adversarial protocol and backward-compatibility tests.
- Do not add HTTP upload, file selection, Tauri commands, settings UI, BUD-03 discovery, or Publisher Studio UI.

Likely files:

- `core/src/blossom.rs`
- `core/src/lib.rs`
- `core/src/store_page.rs`
- `core/src/store_page_content_policy.rs`
- focused core test modules

Validation:

```bash
cargo test -p arcadestr-core blossom
cargo test -p arcadestr-core store_page
cargo check -p arcadestr-core
cargo fmt --all -- --check
git diff --check
```

Dependencies: none.

### Phase 2: Native Upload Service

Scope:

- Add opaque native file-selection records, MIME inspection, size enforcement, streaming hash, streamed HTTP upload, bounded responses, descriptor verification, cancellation, and retry primitives.
- Reuse or extract neutral signer-resolution and account-check helpers.
- Do not implement a custom local video preview resource handler.
- Use an existing narrow safe image-preview mechanism only if it meets path and memory constraints; otherwise defer pre-upload preview.
- Keep this phase independent of Store Page UI.

Likely files:

- `desktop/src/blossom_upload.rs`
- `desktop/src/signer.rs` if shared signer helpers are extracted
- `desktop/Cargo.toml`
- focused desktop test modules

Validation:

```bash
cargo test -p arcadestr-desktop blossom_upload
cargo check -p arcadestr-desktop
cargo fmt --all -- --check
git diff --check
```

Dependencies: Phase 1.

### Phase 3: Settings and Server Configuration

Scope:

- Persist account-keyed user-configured Blossom server origins.
- Add deterministic selection resolution and server URL/network validation.
- Add file-specific BUD-06 preflight state without general capability claims.
- Invalidate selected server state on account changes.
- Do not query, parse, or automatically use kind `10063` events in this phase.
- Do not add an application default server.

Likely files:

- `desktop/src/blossom_settings.rs`
- `desktop/src/main.rs` or a refactored settings module
- desktop settings tests

Validation:

```bash
cargo test -p arcadestr-desktop blossom_settings
cargo check -p arcadestr-desktop
cargo fmt --all -- --check
git diff --check
```

Dependencies: Phases 1 and 2.

### Phase 4: Tauri Commands and Progress

Scope:

- Add native picker, inspect, upload, cancel, retry, and server-settings commands.
- Register commands and managed upload state.
- Add correlated progress events and app bridge payloads/listeners.
- Add explicit standalone-web fallbacks.
- Review Tauri capabilities without granting broad filesystem or network access.

Likely files:

- `desktop/src/blossom_commands.rs`
- `desktop/src/main.rs`
- `desktop/capabilities/default.json` only if a narrowly required permission changes
- `app/src/tauri_bridge.rs`

Validation:

```bash
cargo test -p arcadestr-desktop blossom
cargo test -p arcadestr-app tauri_bridge
cargo check -p arcadestr-desktop
cargo check -p arcadestr-app
cargo fmt --all -- --check
git diff --check
```

Dependencies: Phases 1 through 3.

### Phase 5: Store Page Media UI

Scope:

- Add the focused upload dialog/component and minimal styles.
- Integrate upload and manual URL choices into the existing media section.
- Insert verified integrity metadata into the structured draft.
- Add post-upload HTTPS video preview.
- Preserve account-generation protection and existing media editing behavior.
- Keep unrelated Publisher Studio sections unchanged.

Likely files:

- `app/src/ui_v2/views/store_page_publish.rs`
- a focused component under `app/src/ui_v2/components/` if needed
- `app/src/tauri_bridge.rs`
- existing Publisher Studio style module

Validation:

```bash
cargo test -p arcadestr-app store_page
cargo check -p arcadestr-app
cargo check -p arcadestr-web --features web --target wasm32-unknown-unknown
cargo fmt --all -- --check
git diff --check
```

Run `trunk build` from `web/` for the standalone web fallback.

Dependencies: Phases 1 through 4.

### Phase 6: Live Smoke Test and Review

Scope:

- Run local Blossom and relay fixtures.
- Start the desktop application with:

```bash
cargo tauri dev -- -- --relay ws://localhost:10548 --relay ws://localhost:10547
```

- Use Tauri MCP for DOM snapshots, upload progress, cancellation, retry, account switching, visual desktop/narrow checks, and console-log inspection.
- Native file-picker interaction may require manual selection.
- Smoke-test successful upload, duplicate upload, malformed descriptor, cancellation, NIP-46 rejection, account switching, verified draft insertion, and unchanged manual URL entry.
- Do not publish development events or media references to public relays. If publication is needed for validation, use only the local relays.

Validation:

```bash
cargo test -p arcadestr-core
cargo test -p arcadestr-desktop
cargo test -p arcadestr-app
cargo fmt --all -- --check
git diff --check
```

Dependencies: Phases 1 through 5.

## 10. Resolved Product Decisions

- Defer BUD-03 discovery until mirroring or fallback support is implemented.
- Support JPEG, PNG, WebP, MP4, and WebM initially.
- Defer GIF support.
- Limit JPEG, PNG, and WebP files to 20 MiB each.
- Limit MP4 and WebM files to 500 MiB each.
- Ship with no default Blossom server.
- Require a publisher-configured or explicitly selected server.
- Defer a custom local video preview handler unless later testing demonstrates a concrete need.

### Intentional V1 Limitations

- No BUD-03 server-list discovery or automatic fallback.
- No mirroring, remote blob deletion, or media transcoding.
- No persistent upload history.
- No application-default Blossom server.
- No local video preview before upload; video preview uses the verified HTTPS URL afterward.

## Phase 1 Task Reference

Use the following instruction for the next implementation task:

> Read `Arcadestr_Blossom_Media_Upload_Implementation_Plan.md` and implement only Phase 1: Blossom protocol types and the backward-compatible Store Page media schema extension.
