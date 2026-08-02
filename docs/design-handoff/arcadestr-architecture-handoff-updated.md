# Arcadestr — Architecture Handoff (Updated)

## What Arcadestr is

Arcadestr is a desktop game store, game library, installer, and developer publishing application built on Nostr.

It is not primarily a generic classifieds marketplace. NIP-99 listings and NIP-102 receipts are protocol components used inside a broader game-distribution product.

The product supports two main user roles:

### Players

- discover games;
- view full store pages;
- acquire public, paid, gated, or timed-access games;
- claim temporary free campaigns;
- pay Lightning invoices where required;
- receive protocol-backed acquisition or entitlement state;
- download and install game packages;
- launch and manage installed games;
- view purchase and acquisition history;
- use achievements, profiles, and community features where implemented.

### Developers and publishers

- create and publish games;
- manage game metadata;
- build complete store pages;
- upload artwork and video through Blossom media servers;
- configure acquisition policy and pricing;
- create and cancel campaigns;
- publish updates through Nostr;
- inspect validation, signing, relay, media, and publication failures.

---

## Core stack

- **Rust workspace** — shared protocol, application, desktop, and web code
- **Tauri v2** — native desktop shell and command bridge
- **Leptos 0.8** — Rust reactive frontend compiled for the Tauri webview and web target
- **nostr-sdk 0.44** — relay communication, event creation, subscriptions, signing, and protocol integration
- **SQLite** — local persistence, cache, account-scoped state, acquisition state, and migrations
- **Trunk** — Leptos web build pipeline used by the Tauri frontend
- **Blossom-compatible servers** — decentralized media upload and retrieval
- **Lightning / NWC-related integrations** — payment flows where required

The primary development target is the Tauri desktop application. The Leptos frontend may also compile for the web, but design and product behavior should be evaluated as a desktop application first.

---

## Workspace layout

The repository is a multi-crate Rust workspace. Relevant areas include:

- `core/` — protocol models, Nostr event logic, acquisition and entitlement rules, ADP client behavior, validation, and shared domain primitives
- `app/` — application-level models and business logic
- `desktop/` — Tauri entry point, native commands, installer integration, account handling, media upload, and desktop-only behavior
- `web/` — Leptos frontend, routes, UI components, Tailwind styles, and application state
- `adp-server/` — separate fulfillment or authorization service implementation where used by Arcadestr acquisition flows

Exact module locations may evolve. Preserve separation between protocol-facing models, application models, and UI state.

---

## Current application routes and product surfaces

The current UI is organized around these primary routes:

- `/` — Home
- `/browse` — game discovery
- `/game/$id` — game detail and store page
- `/library` — owned, claimed, public, timed-access, and installed games
- `/community` — community and achievement surfaces
- `/publish` — developer publishing and Store Page management
- `/accounts` — account and signer management
- `/purchases` — purchase and acquisition history
- `/settings` — relay, Blossom, download, account, and application settings

The route structure may be redesigned, but these functional areas represent the current product scope.

---

## Game listing and store-page model

Games are discovered and published through Nostr listing events, currently based on **NIP-99 classified listings, kind 30402**.

NIP-99 is used as a transport and discovery representation for games. The UI should not expose Arcadestr as a generic classifieds application merely because the protocol primitive is a classified listing.

A game listing may include or reference:

- title;
- short and long descriptions;
- price;
- categories or tags;
- developer or publisher identity;
- media URLs;
- acquisition policy;
- campaign pointers;
- package, build, or fulfillment metadata;
- platform information;
- publication identifiers and timestamps.

Historically, there have been separate protocol-facing and application-facing game listing structs. Their tag and field alignment must remain synchronized. Changes to one representation require verification across:

- Nostr tag serialization;
- event parsing;
- application models;
- filters;
- UI assumptions;
- local persistence;
- tests and fixtures.

The previous NIP-15 listing model has been migrated to NIP-99. NIP-15 assumptions must not be reintroduced.

---

## Discovery and relay behavior

Game discovery is relay-backed and therefore asynchronous, partial, and eventually consistent.

The system must support:

- cached initial results;
- live relay refresh;
- multiple relays with different response times;
- unavailable or malformed relay responses;
- partial discovery;
- duplicate event resolution;
- replaceable event semantics;
- stale-request rejection;
- account and route changes while requests are in flight.

`MarketplaceFilter` or its successor must remain compatible with the current listing tag shape. Any protocol or metadata change that affects filtering requires explicit validation.

The UI must not claim that a result set is globally complete.

---

## Acquisition policies

Arcadestr supports multiple game access models.

### Public

The game may be downloaded without ownership credentials and, where policy permits, without authentication.

The desktop installer must evaluate the current signed acquisition policy before requiring credentials. Public access must not fail merely because no account is active.

### Gated

The game requires ownership, entitlement, token, NIP-98, or another valid authorization before download.

### Paid

The buyer creates an order, pays a Lightning invoice, and receives the required receipt or entitlement state.

### Timed access

Access is valid only for a defined interval. The UI and installer must distinguish temporary authorization from permanent ownership.

### Campaign-based free claim

A developer may publish a campaign that temporarily allows users to claim an entitlement at no monetary cost.

A zero-price claim is not equivalent to anonymous public access. It may still require:

- an active account;
- a signed claim;
- campaign validation;
- entitlement publication;
- acquisition-state persistence.

---

## Free acquisitions, campaigns, and entitlements

Arcadestr and the related ADP protocol work include free acquisition and campaign support.

Current protocol concepts include:

- **Entitlement Grant** — experimental kind `1030`
- **ADP Campaign** — experimental kind `1031`
- **Authorization lifecycle** — kind `30406`
- listing acquisition policy values such as Public, Gated, and TimedAccess
- optional listing pointers to active campaigns
- claim uniqueness by buyer, game coordinate, and campaign identifier
- pending and published claim states

Relevant application or desktop commands include flows equivalent to:

- discovering campaigns;
- claiming an entitlement;
- updating a listing’s campaign pointer;
- cancelling a campaign.

Campaign cancellation is prospective:

- its effective time is no earlier than the cancellation event timestamp;
- credentials or entitlements validly created before cancellation may remain valid;
- credentials created at or after cancellation are invalid;
- the UI must not imply retroactive deletion of protocol history.

Authorization references should preserve enough information for portable verification, including the authorization root and authorized key where required by the current protocol design.

---

## Paid orders and NIP-102 receipts

Arcadestr uses **NIP-102 Marketplace Receipts**, currently represented by kind `1020`, for paid acquisition and order-state evidence.

Important properties:

- an order is identified by an `o` tag containing an order UUID;
- private receipt content is NIP-44 encrypted between the relevant parties;
- payment proof may include both the BOLT-11 invoice and preimage;
- status updates form an append-only chain through `e`-tag references;
- reviews may be linked to a completed and proven purchase;
- receipt history should be interpreted as an immutable event timeline.

The application must distinguish:

- invoice creation;
- payment detection;
- payment proof;
- receipt publication;
- fulfillment or entitlement publication;
- download authorization;
- installation completion.

These are related but not interchangeable states.

Payment architecture may continue evolving. Designs and code must not assume a centralized cart, centralized payment processor, or mutable server-side order record.

---

## Game packages, download, and installation

Arcadestr includes a desktop installer rather than stopping at acquisition.

The installer flow may involve:

1. resolving the current listing and acquisition policy;
2. determining whether authentication or ownership proof is required;
3. resolving package or fulfillment metadata;
4. obtaining authorized download URLs or credentials;
5. downloading package data;
6. validating size, type, hashes, or descriptors;
7. extracting or installing files;
8. persisting installation metadata;
9. launching the configured game executable or command.

Important implementation behaviors include:

- anonymous download for current public or timed-access policies where allowed;
- credential enforcement for gated content;
- cancellation and retry;
- stale-response rejection after account changes;
- cleanup of aborted operations;
- streamed hashing;
- file mutation detection;
- bounded response sizes;
- redirect and SSRF protection;
- concurrency control for one selected package;
- safe handling of native file pickers;
- late-result rejection.

Acquisition, download, installation, update, and launch must remain separate state machines.

---

## Store Page editor

The developer workflow includes a structured Store Page editor with multiple sections or tabs.

It manages the customer-facing presentation of a game and may include:

- basic identity;
- descriptions;
- capsule and hero art;
- screenshots;
- trailers or video;
- categories and tags;
- platform and build details;
- system requirements;
- links and support information;
- acquisition policy;
- pricing;
- preview;
- validation;
- publication state.

The editor should preserve:

- local drafts;
- unsaved-change state;
- field-level validation;
- preview behavior;
- media integrity metadata;
- distinction between local edits and published events;
- account ownership checks;
- relay publication progress and failure handling.

A redesign must not reduce this to a generic title/description/price form.

---

## Blossom media uploads

Arcadestr uses Blossom-compatible servers for store-page media rather than assuming centralized S3-style storage.

Current media upload scope includes:

- Nostr upload authorization using relevant Blossom/Nostr events such as kinds `24242` and `10063` where applicable;
- HTTPS origin validation;
- JPEG, PNG, and WebP image support;
- MP4 and WebM video support;
- GIF rejection;
- image size limits around 20 MiB;
- video size limits around 500 MiB;
- optional `sha256`, `mime_type`, and `size` metadata;
- temporary file-selection registry state;
- upload progress events;
- per-account signer use;
- configurable Blossom server settings;
- manual media URL replacement.

Security and correctness requirements include:

- streamed hashing of uploaded bytes;
- detection of file changes between selection and upload;
- exact descriptor validation;
- bounded server responses;
- rejection of unsafe redirects;
- SSRF protections, including IPv6 cases;
- DNS timeout and cancellation;
- cancellation-safe cleanup;
- prevention of concurrent uploads for the same selection;
- clearing integrity metadata whenever a URL is manually replaced;
- rejecting stale results after logout or account switching.

The design must expose upload progress, cancellation, retry, duplicate success, payment-required responses, authentication failures, validation failures, and unsafe-server errors.

---

## Accounts and signer lifecycle

Arcadestr supports account-scoped Nostr identity and may use local or remote signers.

Relevant concerns include:

- active account selection;
- NIP-46 signer sessions;
- logout;
- account switching;
- stale signer prevention;
- cancellation of account-bound operations;
- ownership-scoped publication;
- account-scoped purchases, entitlements, campaigns, and library state.

No operation may silently continue using a signer from a previous account after logout or account switching.

Long-running upload, acquisition, publication, picker, and installation operations must reject stale results when their originating account or selection is no longer current.

---

## Identity, profiles, badges, and reviews

Arcadestr may present portable identity and trust features including:

- NIP-05 identity;
- portable gamertag concepts associated with NIP-49/NIP-05 work;
- NIP-58 achievement badges;
- curated badge showcase;
- verified purchase-linked review history;
- developer and buyer identity context.

AI-assisted identity or badge features are planned around a strict gate-and-approve workflow:

1. generate a proposal;
2. show it to the user;
3. require explicit approval;
4. sign or publish only after approval.

No AI-generated profile, badge selection, game metadata, or public content should auto-publish.

---

## Community state

Community functionality is incomplete and must remain honest.

The current UI should not fabricate:

- social feeds;
- player activity;
- comments;
- followers;
- metrics;
- online users;
- engagement counts;
- community images or controls without backing data.

An unavailable or limited state is preferable to fake content.

---

## Local persistence and migrations

SQLite is used for local cache and application state.

Local persistence may include:

- discovered listings;
- account information;
- purchases and receipts;
- entitlements;
- installation state;
- media settings;
- campaign or publication metadata;
- operation recovery state.

Schema changes must remain migration-safe and idempotent. Protocol event history remains authoritative for portable verification, while SQLite provides local indexing, cache, and workflow state.

Do not treat local database rows as globally authoritative protocol state.

---

## Publishing semantics

Nostr publishing is asynchronous and may partially succeed across relays.

Publishing workflows must represent:

- local draft;
- validation;
- signing;
- event construction;
- relay submission;
- partial acceptance;
- complete acceptance according to the application’s threshold;
- retry;
- stale or superseded event state.

For replaceable events, “editing” usually means publishing a newer event rather than mutating an existing centralized record.

For regular non-replaceable events, history remains append-only.

The UI must not obscure these differences when they materially affect user expectations.

---

## Trust boundaries and verification

The application should preserve explicit boundaries between:

- relay data and locally cached data;
- signed events and unsigned UI state;
- payment proof and claimed payment;
- valid entitlement and local ownership assumptions;
- public access and authenticated access;
- active and expired timed access;
- current and revoked authorization;
- uploaded media URL and verified media descriptor;
- local draft and published store page.

All included proofs required by a workflow must validate. Partial proof validation must not be presented as complete verification.

---

## Current design and implementation constraints

The current frontend uses Tailwind with an existing Noir OKLCH token system. A redesign may replace or evolve the visual direction, but it should work within the actual Leptos/Tailwind application rather than assuming a separate React or static-site implementation.

The application already contains real workflows and edge-case handling. A design handoff must preserve them even when the visual composition changes substantially.

Important non-negotiable categories include:

- relay latency and partial results;
- account switching safety;
- public anonymous acquisition;
- gated authorization;
- Lightning payment states;
- append-only receipt history;
- campaign lifecycle;
- entitlement claims;
- Blossom upload integrity;
- installation progress, cancellation, and retry;
- honest unavailable states;
- desktop keyboard and focus behavior.

---

## Known open or evolving areas

The following areas may still evolve and should not be treated as final protocol guarantees:

- exact payment-provider architecture;
- per-order payment-address experiments;
- final portable gamertag protocol details;
- final AI-assisted profile and badge workflows;
- broader community functionality;
- some campaign and authorization protocol details;
- exact package/build publication model.

Designs should leave room for these areas without inventing completed backend behavior.

---

## Handoff rule

Treat the existing Arcadestr game-store application as the product being redesigned.

Do not reinterpret the repository as a generic NIP-99 marketplace solely because it contains listings, receipts, buyer/seller identities, or Lightning payment code. Those are supporting protocol systems inside the game-store product.
