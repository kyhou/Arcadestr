# Arcadestr — Design Spec (Updated Handoff)

## Product direction

Arcadestr is a native-feeling desktop game store and publishing platform built with Tauri and Leptos. It uses Nostr for discovery, identity, publishing, acquisition, receipts, and entitlement-related protocol flows.

This is not a generic classifieds marketplace. The primary product model is:

- players discover, acquire, install, and manage games;
- developers publish games and manage their store presence;
- game access may be public, paid, or temporarily free;
- ownership and acquisition state are proven through protocol events rather than a centralized store database.

Design for a desktop application window, not a responsive marketing website. Mobile breakpoints are not required. Fixed application chrome, keyboard navigation, desktop density, native dialogs, and resizable content regions are appropriate.

## Design objective

Create a coherent game-store experience that feels purpose-built for Arcadestr rather than copied from Steam, Epic Games Store, or a generic web marketplace.

The design may freely redefine:

- visual identity;
- information hierarchy;
- navigation structure;
- layout system;
- component composition;
- typography;
- spacing;
- interaction patterns;
- use of artwork, gradients, motion, and decorative elements.

The design must preserve the functional scope and behavioral rules described below. Do not remove, merge away, or replace existing features with fabricated alternatives.

Game artwork, screenshots, videos, banners, and hero imagery may be represented with placeholders in the mockup. Their absence from the current implementation is not a product limitation and must not drive the design toward text-only layouts.

---

## Primary application structure

The design should cover these major areas:

1. **Home / Discovery**
2. **Browse**
3. **Game detail / Store page**
4. **Library and installation**
5. **Purchases and acquisition history**
6. **Community and achievements**
7. **Developer publishing**
8. **Store Page editor**
9. **Campaign management**
10. **Accounts**
11. **Settings**

Suggested route mapping from the current application:

- `/` — Home
- `/browse` — Browse
- `/game/$id` — Game detail
- `/library` — Library
- `/community` — Community
- `/publish` — Developer publishing
- `/purchases` — Purchases
- `/accounts` — Accounts
- `/settings` — Settings

The final navigation may be reorganized, but every functional area must remain reachable and understandable.

---

## 1. Home / Discovery

The home screen should act as the game-store landing surface.

It may include:

- featured games;
- newly published games;
- recently updated games;
- free or temporarily free games;
- games already owned by the current account;
- curated or protocol-backed sections;
- continuation points for recently installed or played games where real data exists.

The design must account for relay-backed discovery behavior:

- initial loading;
- progressive results;
- slow or unavailable relays;
- partial results;
- stale cached results while refresh continues;
- no-results states;
- failed media loading.

Do not imply that discovery is globally complete or instant.

Game cards should support, when available:

- title;
- artwork or placeholder media;
- developer or publisher identity;
- price or free status;
- acquisition policy;
- ownership or installation state;
- campaign status;
- platform or build availability;
- relevant trust or verification indicators.

Do not display invented ratings, player counts, review totals, discounts, recommendations, or popularity metrics unless backed by real application data.

---

## 2. Browse

Browse is the complete game discovery surface.

It should support the filters and sorting capabilities actually available in the application and protocol model. Do not introduce unsupported filters merely because they are common in other stores.

Possible supported controls include:

- search;
- category or tag filtering;
- acquisition type;
- free versus paid;
- developer or publisher;
- ownership state;
- installation state;
- campaign availability;
- sort order supported by the local model.

Relay and cache state should remain visible without overwhelming the primary task.

Loading, partial-result, empty, stale, and error states are first-class design states.

---

## 3. Game detail / Store page

The game detail screen is the central customer-facing store page.

It should support the complete published game presentation, including fields that may be managed through the Store Page editor:

- game title;
- short and long descriptions;
- hero or capsule artwork;
- image gallery;
- trailer or video media;
- developer and publisher identity;
- categories, tags, and supported platforms;
- release or availability information;
- system requirements where available;
- external links where supported;
- acquisition policy;
- current price;
- active free-access campaign information;
- ownership status;
- installation state;
- achievements or badges where implemented;
- verified purchase or receipt-linked trust information where available.

Primary actions depend on state and must not be collapsed into a single generic button.

Examples:

- Get;
- Claim;
- Buy;
- Install;
- Update;
- Play;
- Repair;
- Uninstall;
- Sign in to acquire;
- View purchase;
- View campaign status.

The page must clearly distinguish:

- public games available without authentication;
- gated games requiring ownership or authorization;
- paid games requiring purchase;
- timed-access games;
- games temporarily free through a campaign;
- owned but not installed games;
- installed games;
- unavailable or invalid listings.

Do not treat missing mockup artwork as evidence that the production design should omit media-heavy presentation.

---

## 4. Acquisition and checkout

Arcadestr does not use a conventional multi-item shopping cart. Acquisition happens per game.

The design must support these acquisition paths:

### Public acquisition

- No purchase is required.
- Anonymous download may be allowed by policy.
- Authentication must not be shown as mandatory when the current acquisition policy permits anonymous access.

### Free entitlement claim

- A signed entitlement or acquisition claim may be required.
- Active campaign state must be shown clearly.
- Claiming may require an account even when the price is zero.
- Claim success should lead to ownership/library state, not a fake checkout receipt.

### Paid acquisition

- An order is created for one game.
- A BOLT-11 Lightning invoice is generated.
- The UI should support QR display, invoice copy, payment detection, expiration, cancellation, retry, and failure.
- Payment proof and receipt state must remain explicit.

### Timed access

- The UI must show the access window and whether access is active, upcoming, or expired.
- Do not present timed access as permanent ownership.

The language must remain technically honest:

- use “Waiting for payment” rather than pretending a centralized processor is handling the order;
- distinguish payment detected from entitlement published;
- distinguish ownership from temporary access;
- distinguish local installation from acquisition.

---

## 5. Purchases and receipt timeline

The Purchases area should present purchase and acquisition history.

Paid order status is based on append-only NIP-102 receipt events. The interface should read as an immutable event chain or timeline, not an editable order record.

Possible stages include:

- order created;
- invoice issued;
- payment confirmed;
- entitlement or fulfillment published;
- download authorized;
- completed;
- cancelled or failed;
- review submitted.

Each event may include:

- timestamp;
- event status;
- counterparty identity;
- proof or verification state;
- related event references;
- diagnostic information suitable for the user;
- expandable technical details where useful.

Free claims and public acquisitions should appear in acquisition history without being falsely presented as paid purchases.

The UI must distinguish:

- payment proof;
- receipt publication;
- entitlement or authorization;
- local download and installation state.

---

## 6. Library and installation

The Library is the player’s owned, claimed, timed-access, and publicly installed game collection.

It should support:

- owned games;
- claimed games;
- public games added or installed anonymously where applicable;
- timed-access games with expiration state;
- installed and uninstalled states;
- active downloads;
- paused, cancelled, failed, and retryable installations;
- update availability;
- launch state;
- repair or verification where implemented;
- local storage and install-location information where available.

The library must not assume that acquisition, download, installation, and launch are a single atomic operation.

Important states include:

- acquiring authorization;
- waiting for credentials;
- resolving media or package URLs;
- downloading;
- verifying hashes;
- extracting or installing;
- ready to play;
- update available;
- missing files;
- access expired;
- authentication required for gated content;
- anonymous access allowed for public content.

Long-running operations must include clear progress, cancellation, retry, and failure recovery.

---

## 7. Community and achievements

Community functionality must reflect real implemented data.

Where the feature is incomplete or unavailable, show an honest unavailable or limited state rather than fabricated feeds, activity, followers, player counts, or engagement statistics.

Achievements and profile badge presentation may include:

- earned badges;
- badge details;
- showcase selection;
- verified issuer information;
- relay refresh state;
- empty and unavailable states.

NIP-58 badges are user-curated. The design should distinguish earned badges from badges selected for display.

AI-assisted identity or badge features, when present, must use a gate-and-approve pattern:

1. the system proposes content or a change;
2. the user reviews it;
3. the user explicitly approves publication;
4. nothing is published automatically.

---

## 8. Developer publishing

The Publish area is a developer-facing workspace, not merely a generic listing form.

It should support:

- viewing games published by the current account;
- creating a new game;
- editing game metadata;
- managing drafts and published games;
- validating required fields;
- publishing protocol events;
- showing relay publication progress and partial failure;
- managing builds or downloadable artifacts where implemented;
- opening the Store Page editor;
- managing media;
- managing acquisition policy;
- managing campaigns;
- reviewing publication status and errors.

The design must account for account ownership. Only games associated with the active developer identity should appear as editable.

Publishing is not instantaneous. Show:

- local draft state;
- validation state;
- signing state;
- relay publication progress;
- partial relay acceptance;
- complete publication;
- retryable failures;
- stale local-versus-relay state.

Editing a live game generally republishes protocol events. Avoid language implying centralized in-place mutation of a canonical database record.

---

## 9. Store Page editor

The Store Page editor is a structured game-presentation workflow.

The current implementation contains multiple editing sections or tabs and should remain a full editor rather than being reduced to one generic form.

The design should accommodate fields such as:

- identity and basic details;
- descriptions;
- branding and artwork;
- screenshots and video;
- categories and tags;
- platform/build information;
- system requirements;
- links and support information;
- acquisition and pricing;
- preview and validation.

Exact field grouping may change, but all existing editable information must remain accessible.

The editor should support:

- persistent draft state;
- unsaved-change indication;
- field-level validation;
- preview mode;
- media upload state;
- publication readiness;
- safe navigation when changes are pending;
- clear separation between local edits and published state.

The preview should approximate the final store page using placeholder media where required.

---

## 10. Blossom media uploads

Game artwork and video are uploaded to Blossom-compatible media servers rather than assumed to live in centralized application storage.

The UI should support:

- selecting or changing a Blossom server;
- image uploads for supported formats;
- video uploads for supported formats;
- upload progress;
- cancellation;
- retry;
- duplicate upload handling;
- integrity verification;
- MIME/type mismatch errors;
- file mutation or hash mismatch errors;
- oversized file errors;
- authentication or authorization errors;
- payment-required responses where applicable;
- redirect rejection or unsafe-server errors;
- manual media URL replacement;
- clearing stale integrity metadata when a URL is manually replaced.

Current constraints include:

- JPEG, PNG, and WebP images;
- MP4 and WebM video;
- GIF rejection;
- bounded file sizes;
- streamed hashing and upload validation;
- authenticated upload flows where required.

Do not hide technical failures behind a generic “Something went wrong” state. Present concise user-facing explanations with optional technical details.

---

## 11. Campaign management

Developers can create and manage acquisition campaigns for their games.

The design should support:

- selecting one of the current developer’s games;
- viewing campaigns for that game;
- creating a campaign;
- editing where protocol rules allow;
- cancelling a campaign;
- showing active, upcoming, ended, cancelled, and invalid states;
- automatic campaign identifiers;
- date and time pickers;
- campaign type;
- free-claim campaigns;
- price-related fields only when supported by the selected campaign type;
- publication and relay state;
- campaign pointer updates on the listing where applicable.

Campaign cancellation is prospective. Existing valid credentials or entitlements created before the effective cancellation time may remain valid according to protocol rules. The UI must not imply that cancellation retroactively revokes all prior acquisitions.

Do not expose raw UNIX timestamp fields or protocol booleans as the primary interaction model when standard desktop controls can represent them safely.

---

## 12. Accounts and identity

Arcadestr may operate with multiple Nostr identities and signing methods.

The Accounts area should support, where implemented:

- active account selection;
- local or external signer state;
- NIP-46 connection state;
- profile identity;
- portable gamertag or NIP-05 identity;
- account-specific library, purchases, publications, and settings;
- sign-in and sign-out;
- connection errors;
- expired or stale signer sessions.

Account switching must be visually explicit because ownership, publishing rights, entitlements, campaigns, and purchase history are account-scoped.

The UI must not reuse stale signer or authorization state after logout or account switching.

---

## 13. Settings

Settings may include:

- relay configuration;
- Blossom media server configuration;
- download and installation preferences;
- storage locations;
- account and signer preferences;
- appearance;
- diagnostics;
- protocol or developer settings where appropriate.

Settings changes should show whether they are:

- local-only;
- account-specific;
- published to Nostr;
- dependent on reconnection or restart.

---

## Cross-cutting behavioral rules

### Preserve real functionality

Do not simplify the product by removing difficult states or replacing real workflows with decorative mock controls.

### Do not fabricate data

Do not invent:

- ratings;
- review counts;
- player counts;
- followers;
- activity feeds;
- download counts;
- discounts;
- sales rankings;
- editorial recommendations;
- social metrics;
- online status;
- unsupported protocol guarantees.

Placeholder text and artwork are acceptable in design mockups when clearly representational.

### Treat latency as normal

Relay operations, signing, publishing, Lightning payment, entitlement creation, Blossom upload, download, and installation may all take time or partially fail.

Pending and recovery states must be designed deliberately.

### Distinguish trust states

The visual system should differentiate:

- verified versus unverified;
- signed versus unsigned;
- paid versus unpaid;
- owned versus merely available;
- permanent ownership versus timed access;
- published versus local draft;
- confirmed versus pending;
- valid versus revoked or expired;
- protocol proof versus local cache state.

### Preserve privacy and protocol semantics

Do not expose encrypted receipt content publicly. Do not imply centralized custody, centralized account ownership, or irreversible server authority where the application uses peer-to-peer protocol events.

### Use honest actions

Button labels and status text must describe the actual operation. Avoid generic labels such as “Continue” where “Publish,” “Claim,” “Pay invoice,” “Install,” or “Retry upload” is more accurate.

### Desktop accessibility

Support:

- keyboard navigation;
- visible focus states;
- correct focus trapping in modals;
- focus restoration after dialogs close;
- readable contrast;
- scalable text;
- non-color-only status indicators;
- resizable-window behavior;
- overflow handling at smaller desktop window sizes.

---

## Visual direction

No visual identity is mandatory. Claude Design may propose a new direction.

The visual language should communicate:

- games and discovery;
- developer ownership;
- protocol-backed trust;
- user-controlled identity;
- decentralized publishing without turning the interface into a technical dashboard.

Avoid making every screen look like a terminal merely because the backend uses Nostr and Lightning. Protocol details should be available where useful, but the primary experience should remain understandable to players and game developers.

The result should feel like one coherent desktop product across consumer, library, purchase, and developer workflows.
