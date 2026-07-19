**Current screens, missing screens, and user-facing information requirements**

*Product and UX planning document | Revised July 2026*

# Document purpose

This document defines the complete user-facing screen set for Arcadestr. It separates screens that already exist, screens that exist but require expansion, and screens that need to be created. Technical protocol details are translated into language that players and game developers can understand.


# Free-acquisition model used by this document

Arcadestr has four distinct acquisition paths. They must not be merged into one generic “free purchase” flow.

| Path | User result | Durable account record | Correct primary action |
|---|---|---:|---|
| Paid purchase | Permanent access backed by a purchase record | Yes | Buy |
| Public access | Access only while the current game page allows everyone to download | No | Install or Download |
| Timed access | Access only during the displayed start/end period | No | Install while available |
| Claim-and-keep promotion | Permanent access after the signed-in user claims an active promotion | Yes | Claim game |

A zero or missing price never means public access. Public and timed access must be explicitly enabled by the developer. Cancelling a claim-and-keep promotion blocks later claims but does not remove access already granted.

# Product language rules

- Use “account”, “identity”, or “signing app” instead of protocol names in primary UI copy.

- Use “game access”, “purchase record”, and “download permission” instead of event kinds, coordinates, or cryptographic proof terminology.

- Show technical identifiers only in an optional “Technical details” section.

- Every network action needs a clear progress state, success result, recoverable error, and retry action.

- Player flows and developer flows should be visually separated. Developer controls should not clutter the store experience.

- Destructive actions require a confirmation dialog that states the consequence in plain language.

# Recommended primary navigation

| **Area**  | **Visible to**    | **Primary destinations**                       | **Notes**                                                     |
|-----------|-------------------|------------------------------------------------|---------------------------------------------------------------|
| Store     | Everyone          | Home, Browse, Game details                     | Default area after startup.                                   |
| Library   | Signed-in players | Owned games, installed games, downloads, updates | Separates durable ownership from temporary/public installations. |
| Community | Signed-in users   | Social feed, profiles, achievements            | Can remain secondary until core marketplace flows are mature. |
| Publish   | Developers        | Dashboard, new game, releases, promotions      | Developer mode; should not expose protocol terminology.       |
| Account   | Signed-in users   | Profile, identities, security, network, backup | Consolidates scattered account-management controls.           |

# Status legend

**Existing:** A dedicated screen or view exists in the current UI.

**Partial:** The screen exists, but important fields, actions, or states are missing.

**New:** No complete user-facing screen exists and one should be designed.

**Optional later:** Useful, but not required for the first complete marketplace experience.

# 1. Store and discovery

## 1.1 Store home

| **Status**                     | **Primary user** | **Purpose**                                                           |
|--------------------------------|------------------|-----------------------------------------------------------------------|
| Existing; redesign recommended | All users        | Introduce the marketplace and help users find relevant games quickly. |

Information and fields

**Featured game:** Cover, title, short description, paid price or explicit access label, platform support, and primary action.

**Featured collections:** New releases, free games, popular games, and editor-selected categories.

**Continue playing:** Owned or installed games recently opened; shown only when relevant.

**Category shortcuts:** Genre or experience categories using familiar names.

**Marketplace status:** A subtle offline or limited-connectivity notice when fresh results cannot be loaded.

Main actions

Open a game; Browse a collection; Continue playing; Refresh marketplace.

Required states and feedback

Initial loading skeleton; Loaded with content; No games available; Offline using saved data; Partial network failure.

UX notes

Avoid relay counts, event kinds, and raw network status in the main content area.

## 1.2 Browse games

| **Status**       | **Primary user** | **Purpose**                                 |
|------------------|------------------|---------------------------------------------|
| Existing; expand | All users        | Search and filter the complete marketplace. |

Information and fields

**Search:** Search by game title, developer name, description, and tags.

**Platform:** This device, Windows, Linux, macOS, Web, or All.

**Acquisition:** All, Paid, Public access, Timed access, Claim-and-keep promotion, and Owned. Optional price range applies only to paid games.

**Genre:** Multi-select user-friendly categories.

**Availability:** Available now, Promotion active, Timed access active, Starts later, or Ended.

**Sort:** Recommended, newest, price, and title.

**Game cards:** Cover, title, developer, paid price or explicit label such as Public access, Free until [date], or Claim and keep, plus platform badges and owned/installed state.

Main actions

Search; Apply filters; Clear filters; Open game; Install owned game.

Required states and feedback

Loading; Results; No search results; No compatible games; Offline cached results; Install in progress; Install failed.

UX notes

Keep advanced compatibility details behind an expandable filter rather than showing architecture strings by default.

## 1.3 Game details

| **Status**                         | **Primary user**               | **Purpose**                                                                          |
|------------------------------------|--------------------------------|--------------------------------------------------------------------------------------|
| Existing; major expansion required | Players and prospective buyers | Explain the game, confirm compatibility, and provide the correct acquisition action. |

Information and fields

**Title and developer:** Prominent game identity with developer profile link.

**Media:** Cover image, screenshots, optional trailer, and broken-media fallback.

**Summary and description:** Short overview followed by full description.

**Price and access:** Paid price, Public access, Timed access with clear dates, or Claim-and-keep promotion. Never infer public access from a zero or missing price.

**Primary action:** Buy; Claim game for a claim-and-keep promotion; Install for current public or timed access; Update; Play; Unavailable; or Coming soon.

**Compatibility:** Supported operating systems and device compatibility in plain language.

**Release information:** Version, release date, download size, and last update.

**Game details:** Genre, features, languages, age/content notes, controller support, and multiplayer support.

**Developer information:** Avatar, name, verification status, about text, website, and support contact.

**Ownership status:** Purchased, promotion claimed, installed through temporary/public access, refunded/disputed, access revoked, installed, or update available.

**Technical details:** Optional section with listing ID, file integrity information, and server details.

Main actions

Buy; Claim game; Install while available; Install owned game; Update; Play; Open developer profile; Copy payment request; Open wallet; Retry.

Required states and feedback

Loading; Ready; Payment request loading; Waiting for payment; Payment confirmed; Promotion claim in progress; Game claimed; Public access active; Timed access active; Timed access not started; Timed access ended; Download in progress; Install in progress; Unavailable for this device; No automated download; Media failure; Network failure.

UX notes

Use “Claim game” only for claim-and-keep promotions. Public and timed access do not create ownership records, so their action is “Install” or “Download while available”.

## 1.4 Search results

| **Status** | **Primary user** | **Purpose**                                                                               |
|------------|------------------|-------------------------------------------------------------------------------------------|
| New        | All users        | Provide a focused full-page result experience when search grows beyond the browse header. |

Information and fields

**Query:** Current search text.

**Result groups:** Games first; optionally developers and collections later.

**Filters:** Platform, price, genre, ownership, and availability.

**Result count:** Approximate total when available.

**Recent searches:** Local-only list with clear control.

Main actions

Open result; Refine search; Clear recent searches.

Required states and feedback

Searching; Results; No matches; Network unavailable; Saved results only.

## 1.5 Collection or category page

| **Status** | **Primary user** | **Purpose**                                     |
|------------|------------------|-------------------------------------------------|
| New        | All users        | Present a curated or rule-based group of games. |

Information and fields

**Collection title:** User-friendly name.

**Description:** Why these games are grouped.

**Games:** Standard game cards.

**Sort and filters:** Reduced set appropriate to the collection.

Main actions

Open game; Filter; Share collection later.

Required states and feedback

Loading; Loaded; Empty collection; Offline.

# 2. Purchase and access

## 2.1 Payment dialog

| **Status**                                          | **Primary user**           | **Purpose**                                                  |
|-----------------------------------------------------|----------------------------|--------------------------------------------------------------|
| Partial inside game details; dedicated modal needed | Players buying a paid game | Guide the user from price confirmation to completed payment. |

Information and fields

**Game and amount:** Title, price, and currency.

**Payment request:** QR code and copyable Lightning invoice.

**Expiration:** Human-readable remaining time.

**Wallet action:** Open compatible wallet when available.

**Payment status:** Waiting, detected, confirmed, expired, or failed.

**What happens next:** Explain that access and installation become available after confirmation.

Main actions

Copy payment request; Open wallet; Cancel; Generate a new request; Continue to install.

Required states and feedback

Generating; Waiting for payment; Confirmed; Expired; Cancelled; Verification delayed; Error.

UX notes

Do not expose preimages, zap event IDs, or proof-validation language.

## 2.2 Promotion claim confirmation

| **Status** | **Primary user** | **Purpose** |
|---|---|---|
| New | Signed-in players claiming a claim-and-keep promotion | Confirm that the current account will receive permanent game access. |

Information and fields

**Game:** Title and cover.

**Promotion:** Human-readable promotion name.

**Claim window:** Start and end date/time in the user’s local timezone.

**Access statement:** “Claim this game now and keep it in your library after the promotion ends.”

**Account:** Identity that will receive access, shown as name and avatar.

**Existing access:** State clearly when the account already owns or has claimed the game.

Main actions

Claim game; Cancel.

Required states and feedback

Ready; Waiting for signing approval; Claiming; Claimed and added to library; Promotion not started; Promotion ended; Promotion cancelled; Already owned; Provider unavailable; Network error.

UX notes

This screen is only for claim-and-keep promotions. Public and timed access skip this confirmation because they do not create durable ownership.

## 2.3 Current public or timed access notice

| **Status** | **Primary user** | **Purpose** |
|---|---|---|
| New lightweight panel on Game details | Any player | Explain temporary access without presenting it as ownership. |

Information and fields

**Access type:** Available to everyone or Available for a limited time.

**Availability:** For timed access, show exact start and end dates and whether access is upcoming, active, or ended.

**Durability notice:** “This access is not added permanently to your account.”

**Installed-game consequence:** Explain that future downloads or reinstalls may require another valid access path after the offer ends.

Main actions

Install; Download; Sign in when required for installation management.

Required states and feedback

Available now; Starts later; Ended; Download preparing; Provider unavailable; Device unsupported.

## 2.4 Purchase or claim success

| **Status** | **Primary user** | **Purpose**                                                        |
|------------|------------------|--------------------------------------------------------------------|
| New        | Players          | Provide a clear completion step after a paid purchase or successful promotion claim. |

Information and fields

**Success message:** Game added to library.

**Game summary:** Cover, title, platform, and version.

**Next action:** Install now or view in library.

**Acquisition record:** Human-readable date, Paid purchase or Promotion claim, amount when paid, and status.

**Recovery note:** Access is linked to the signed-in identity and can be recovered on another compatible Arcadestr installation.

Main actions

Install now; Go to library; View purchase receipt or access record.

Required states and feedback

Ready; Install starting; Record publication pending; Claim already existed and was recovered.

## 2.5 Purchase receipt

| **Status** | **Primary user** | **Purpose**                                                            |
|------------|------------------|------------------------------------------------------------------------|
| New        | Players          | Show a readable proof of purchase without exposing protocol internals. |

Information and fields

**Game:** Title and developer.

**Order number:** Shortened, copyable identifier.

**Date:** Purchase date.

**Amount:** Paid amount.

**Status:** Paid, fulfilled, refunded, disputed, or cancelled.

**Account:** Identity that owns the game.

**Payment method:** Lightning.

**Technical details:** Optional full order ID, event ID, seller key, and verification status.

Main actions

Copy order number; Open game; Download receipt as text or PDF later.

Required states and feedback

Verified; Verification pending; Refunded; Disputed; Invalid or incomplete record.

## 2.6 Permanent access record

| **Status** | **Primary user** | **Purpose** |
|---|---|---|
| New | Players who claimed a promotion or received another non-payment grant | Show durable game access separately from a payment receipt. |

Information and fields

**Game:** Title and developer.

**Access type:** Promotion claim, Gift, Review copy, Contest prize, or Migrated access when supported.

**Granted to:** Account name and short public identifier.

**Granted date:** Human-readable local date/time.

**Promotion:** Promotion name and claim period when the access came from a campaign.

**Status:** Active, Verification pending, or Revoked.

**Durability statement:** “This access remains valid after the promotion ends unless the developer revokes this specific access record.”

**Technical details:** Optional grant ID, source promotion ID, issuer, event chain, and verification status.

Main actions

Open game; Install; Copy access-record ID; View technical details.

Required states and feedback

Verified and active; Verification pending; Revoked; Incomplete chain; Conflicting record; Network unavailable.

## 2.7 Acquisition history

| **Status** | **Primary user** | **Purpose**                                                                             |
|------------|------------------|-----------------------------------------------------------------------------------------|
| New        | Players          | List durable paid purchases and promotion claims independently from installed games. |

Information and fields

**Records:** Game, acquisition type, date, amount when paid, status, and developer.

**Filters:** Purchases, Promotion claims, Refunded, Disputed, Revoked access, and date range.

**Search:** Game or developer.

**Totals:** Optional total spent for the selected period.

Main actions

Open purchase receipt or access record; Open game; Filter.

Required states and feedback

Loading; Loaded; Empty; Verification pending; Offline cached history.

# 3. Library and installation

## 3.1 Library

| **Status**       | **Primary user** | **Purpose**                                               |
|------------------|------------------|-----------------------------------------------------------|
| Existing; expand | Players          | Manage all games the current identity owns or can access. |

Information and fields

**Library cards:** Cover, title, installed state, version, platform, and primary action.

**Tabs:** Owned, Installed, Not installed, Updates, and Access problems. Optionally show locally installed public/timed games in Installed without labeling them as owned.

**Search and sort:** Title, recently acquired, recently played, and developer.

**Access source:** Purchased, Promotion claim, Gift/manual grant when supported, or Temporary/public install. Only durable records belong in Owned.

**Storage summary:** Optional total installed size and selected install location.

Main actions

Install; Update; Play; Open game; Repair; Remove local files.

Required states and feedback

Loading; Empty library; Installing; Updating; Installed; Update available; Access verification pending; Promotion access revoked; Temporary access expired; Download source unavailable; Offline.

UX notes

Removing local files must not imply removing ownership.

## 3.2 Download and install progress

| **Status**                   | **Primary user** | **Purpose**                                                 |
|------------------------------|------------------|-------------------------------------------------------------|
| New dedicated modal or panel | Players          | Make long-running downloads understandable and recoverable. |

Information and fields

**Game:** Title, cover, and version.

**Progress:** Downloaded amount, total size, percentage, and speed.

**Current step:** Preparing, downloading, verifying, extracting, or finishing.

**Destination:** Install folder.

**Time estimate:** Only when reliable.

**Integrity result:** Plain-language confirmation that files are valid.

Main actions

Pause later; Cancel; Retry; Open install folder; Launch.

Required states and feedback

Queued; Downloading; Paused; Verifying; Installing; Complete; Cancelled; Network interrupted; Insufficient space; File verification failed; Permission denied.

## 3.3 Installed game management

| **Status** | **Primary user** | **Purpose**                                                    |
|------------|------------------|----------------------------------------------------------------|
| New        | Players          | Manage one installed game without returning to the store page. |

Information and fields

**Installed version:** Current local version.

**Latest version:** Available marketplace version.

**Install location:** Folder path with open action.

**Disk usage:** Installed size.

**Access status:** Valid, needs verification, or unavailable.

**Launch settings:** Optional command or executable selection only when automatic detection fails.

Main actions

Play; Update; Verify files; Open folder; Move later; Uninstall local files.

Required states and feedback

Ready; Update available; Verifying; Repair required; Executable missing; Access check unavailable.

## 3.4 Install location settings

| **Status** | **Primary user** | **Purpose**                                    |
|------------|------------------|------------------------------------------------|
| New        | Desktop users    | Choose default and per-game storage locations. |

Information and fields

**Default library folder:** Current folder and free space.

**Additional folders:** Optional list for multiple drives.

**Per-game override:** Shown during installation.

**Space requirement:** Required and available space before install.

Main actions

Choose folder; Add folder; Remove folder; Set default.

Required states and feedback

Folder valid; Folder unavailable; Insufficient space; Permission denied.

# 4. Identity, profile, and community

## 4.1 Login and account connection

| **Status**                             | **Primary user** | **Purpose**                                                            |
|----------------------------------------|------------------|------------------------------------------------------------------------|
| Existing; simplify and expand recovery | All users        | Connect an identity securely using a signing app or browser extension. |

Information and fields

**Connection method:** Recommended signing app, browser signer, or advanced connection address.

**QR code:** For desktop-to-mobile connection.

**Connection address:** Copyable fallback link.

**Saved accounts:** Existing identities with avatar, display name, and connection status.

**Privacy explanation:** State that Arcadestr requests signatures but does not need the private key.

**Advanced import:** Encrypted key import only in a clearly separated advanced section.

Main actions

Connect; Scan QR; Copy connection link; Use saved account; Remove saved account; Retry.

Required states and feedback

Preparing; Waiting for signer; Approval required; Connected; Rejected; Timed out; Signer unavailable; Restoring session.

UX notes

Direct private-key entry should not be part of the normal production flow.

## 4.2 Account switcher

| **Status**                                                  | **Primary user** | **Purpose**                                                 |
|-------------------------------------------------------------|------------------|-------------------------------------------------------------|
| Existing component; needs complete page or polished popover | Signed-in users  | Switch among saved identities and manage connection status. |

Information and fields

**Accounts:** Avatar, name, short public identifier, active state, and connection state.

**Current account:** Clearly marked.

**Pending requests:** Optional indication when the signer needs attention.

Main actions

Switch; Reconnect; Rename local label; Remove from device; Add account.

Required states and feedback

Connected; Connecting; Disconnected; Signer approval required; Removal confirmation.

## 4.3 Own profile

| **Status**                                       | **Primary user** | **Purpose**                                                     |
|--------------------------------------------------|------------------|-----------------------------------------------------------------|
| Existing; expand editing and account distinction | Signed-in users  | Show the public profile and provide profile-management actions. |

Information and fields

**Avatar and banner:** With fallbacks.

**Display name and handle:** Primary human-readable identity.

**Verification:** Verified identifier status with explanation.

**About:** Biography.

**Website and Lightning address:** Clickable and validated.

**Badges:** Selected achievements.

**Published games:** Games from this developer identity.

**Public identifier:** Short form, copyable; full value under technical details.

Main actions

Edit profile; Copy profile link; Manage badges; Open published game.

Required states and feedback

Loading; Loaded; Profile incomplete; Verification pending; Verification failed; Offline cached profile.

## 4.4 Edit profile

| **Status** | **Primary user** | **Purpose**                                         |
|------------|------------------|-----------------------------------------------------|
| New        | Signed-in users  | Edit public profile metadata with clear validation. |

Information and fields

**Profile picture:** Image URL with preview and fallback.

**Display name:** Primary name.

**Username:** Optional short name.

**About:** Character-limited biography.

**Website:** Validated URL.

**Verified identifier:** Address-like identifier with status check.

**Lightning address:** Validated address for receiving payments.

Main actions

Save changes; Cancel; Verify identifier.

Required states and feedback

Unsaved changes; Saving; Saved; Validation error; Signer approval required; Publish failed.

## 4.5 Public developer profile

| **Status**                        | **Primary user** | **Purpose**                                                |
|-----------------------------------|------------------|------------------------------------------------------------|
| Existing via profile view; refine | All users        | Help buyers evaluate a developer and discover their games. |

Information and fields

**Identity:** Avatar, name, verification, and short identifier.

**About and links:** Biography, website, support contact, and Lightning address.

**Games:** Published active games.

**Badges:** Relevant public achievements.

**Trust indicators:** Account verification and game-history signals; avoid unsupported ratings.

Main actions

Open game; Open website; Copy profile link.

Required states and feedback

Loading; Loaded; No published games; Profile unavailable; Offline cached profile.

## 4.6 Achievements

| **Status**       | **Primary user** | **Purpose**                                   |
|------------------|------------------|-----------------------------------------------|
| Existing; refine | Signed-in users  | Show earned badges and explain their meaning. |

Information and fields

**Badge:** Image, name, description, issuer, and earned date.

**Profile visibility:** Whether shown publicly.

**Progress:** Only for achievements with reliable measurable progress.

**Filters:** Shown on profile and all earned.

Main actions

Show on profile; Hide from profile; Open issuer profile.

Required states and feedback

Loading; Loaded; No achievements; Badge details unavailable.

## 4.7 Social feed

| **Status**                        | **Primary user** | **Purpose**                                                       |
|-----------------------------------|------------------|-------------------------------------------------------------------|
| Existing; optional later redesign | Signed-in users  | Provide community activity without obstructing marketplace tasks. |

Information and fields

**Feed items:** Author, timestamp, content, and referenced game when present.

**Composer:** Text and optional game reference.

**Filters:** Following and marketplace activity later.

**Moderation controls:** Mute, report, and hide.

Main actions

Post; Reply later; Open profile; Open game; Mute; Report.

Required states and feedback

Loading; Loaded; Empty feed; Posting; Post failed; Offline.

# 5. Developer and publishing

## 5.1 Developer dashboard

| **Status** | **Primary user** | **Purpose**                                                                        |
|------------|------------------|------------------------------------------------------------------------------------|
| New        | Game developers  | Summarize published games, release status, access offers, and distribution health. |

Information and fields

**Games:** Title, status, price/access type, latest version, platforms, and distribution state.

**Quick metrics:** Owned/accessed count only when reliable; avoid misleading sales analytics from incomplete relay data.

**Attention items:** Missing download file, unavailable server, expired promotion, invalid payment address, or unpublished changes.

**Recent activity:** Published updates, uploads, access claims, and purchases in plain language.

Main actions

Create game; Edit game; Upload release; Create promotion; Open game page.

Required states and feedback

Loading; No games; Healthy; Attention required; Network data incomplete.

## 5.2 Create game: basic information

| **Status**                                           | **Primary user** | **Purpose**                           |
|------------------------------------------------------|------------------|---------------------------------------|
| Partial in existing Publish screen; split into steps | Game developers  | Create the public store presentation. |

Information and fields

**Game title:** Required.

**Short summary:** Required; concise store-card text.

**Full description:** Required; supports formatting preview.

**Cover image:** Required URL or future upload, with preview and validation.

**Screenshots:** Multiple ordered images with previews.

**Trailer:** Optional video URL.

**Genres and tags:** User-friendly selectable terms plus optional custom tags.

**Website and support:** Optional links.

**Status:** Draft, Published, or Unlisted in application terms.

Main actions

Save draft; Continue; Preview store page; Cancel.

Required states and feedback

Draft saved; Unsaved changes; Image invalid; Validation errors; Signer unavailable.

UX notes

Do not ask the user to choose NIP-99 or enter event kinds.

## 5.3 Create game: pricing and access

| **Status**                                     | **Primary user** | **Purpose**                       |
|------------------------------------------------|------------------|-----------------------------------|
| New step replacing technical campaign controls | Game developers  | Define how users obtain the game. |

Information and fields

**Base selling model:** Paid or No paid purchase option. This controls whether buyers can purchase through Lightning.

**Price:** Amount in sats when paid purchases are enabled. A zero price must not silently enable public access.

**Payment address:** Address that receives purchases, required for paid games.

**Current ungated access:** Gated, Public access, or Timed access. Public and timed access do not grant permanent ownership.

**Timed access start and end:** Required only for Timed access; local date/time with timezone shown.

**Claim-and-keep promotions:** Created separately after the game exists. Explain that users who claim during the active period keep access permanently.

**Result after timed access:** The game returns to its normal gated or paid behavior after the end time. Existing paid purchases and promotion claims remain valid.

Main actions

Continue; Save draft; Preview buyer experience.

Required states and feedback

Valid; Missing payment address; Invalid timed-access range; Zero-price warning; Signer approval required.

## 5.4 Create game: builds and compatibility

| **Status**                         | **Primary user** | **Purpose**                                                  |
|------------------------------------|------------------|--------------------------------------------------------------|
| Partial in Publish; major redesign | Game developers  | Attach downloadable releases and describe supported devices. |

Information and fields

**Release version:** Required semantic version, explained with example.

**Platforms:** Operating system and processor options selected from labels, not typed codes.

**Game archive:** File picker; checksum calculated automatically.

**Download size:** Calculated.

**Distribution provider:** Selected hosting service with human-readable name and status.

**Release notes:** Optional text.

**Minimum requirements:** Optional structured fields: OS, processor, memory, graphics, storage.

**Recommended requirements:** Optional structured fields.

Main actions

Choose file; Choose provider; Upload and verify; Continue; Save draft.

Required states and feedback

Hashing file; Uploading; Verifying; Upload complete; Provider unreachable; File too large; File changed; Platform missing; Upload failed.

UX notes

Never expose a freeform file hash field. Calculate and verify it automatically.

## 5.5 Distribution provider selection

| **Status** | **Primary user** | **Purpose**                                                     |
|------------|------------------|-----------------------------------------------------------------|
| New        | Game developers  | Choose and authorize a service that stores and delivers builds. |

Information and fields

**Provider:** Name, website, status, supported version, and operator identity verification.

**Current authorization:** Connected, needs approval, revoked, or unavailable.

**Scope label:** Generated from game name; hidden unless advanced editing is necessary.

**Redundancy:** Allow selecting more than one provider later.

**Explanation:** State what the provider can do: store builds and issue purchase/access records for this game. It cannot edit the developer profile.

Main actions

Connect provider; Approve authorization; Reconnect; Remove provider; Test connection.

Required states and feedback

Checking; Available; Provisioning; Waiting for signature; Connected; Unavailable; Authorization mismatch; Revoked.

UX notes

Protocol concepts such as fulfillment keys and attestation events belong only in optional technical details.

## 5.6 Review and publish

| **Status**                                    | **Primary user** | **Purpose**                                                      |
|-----------------------------------------------|------------------|------------------------------------------------------------------|
| Partial in Publish; formal review step needed | Game developers  | Show exactly what buyers will see and block invalid publication. |

Information and fields

**Store preview:** Cover, summary, price/access label, platforms, developer identity.

**Release summary:** Version, file size, providers, and upload verification.

**Readiness checklist:** Required fields, payment setup, file uploaded, provider reachable, and account connected.

**Visibility:** Published or Unlisted when supported.

**Change summary:** For edits, clearly list what changed.

Main actions

Publish game; Save draft; Back to edit; Open technical details.

Required states and feedback

Ready; Warnings; Blocking errors; Waiting for signature; Publishing; Published; Partially published; Publish failed.

UX notes

Warnings must distinguish “game can be listed but cannot be installed automatically” from true publication blockers.

## 5.7 Manage game

| **Status** | **Primary user** | **Purpose**                                     |
|------------|------------------|-------------------------------------------------|
| New        | Game developers  | Edit a published game and manage its lifecycle. |

Information and fields

**Overview:** Current public status, price/access, version, platforms, and store link.

**Store information:** Edit metadata and media.

**Releases:** Current and previous builds.

**Access and promotions:** Paid purchase settings, current public/timed policy, claim-and-keep campaign history, and durable grant revocations.

**Distribution:** Providers and upload health.

**Danger zone:** Unlist game, cancel promotion, revoke an individual grant, disable public/timed access, or remove a provider.

Main actions

Edit; Publish changes; Create release; Create promotion; Cancel promotion; Change current access policy; Unlist.

Required states and feedback

Saved; Unpublished changes; Publishing; Published; Conflicting newer update; Network incomplete.

## 5.8 Releases

| **Status** | **Primary user** | **Purpose**                                       |
|------------|------------------|---------------------------------------------------|
| New        | Game developers  | Manage versions and platform builds for one game. |

Information and fields

**Release list:** Version, date, platform, file size, status, and provider availability.

**Release details:** Notes, file verification, and download health.

**Active release:** Clearly marked per platform.

**Older releases:** Retained history when available.

Main actions

Create release; Replace failed upload; Publish release; Deactivate release.

Required states and feedback

Draft; Uploading; Ready; Active; Superseded; Unavailable; Corrupted.

## 5.9 Promotions and free-access campaigns

| **Status** | **Primary user** | **Purpose**                                                |
|------------|------------------|------------------------------------------------------------|
| New        | Game developers  | Create, inspect, update, and cancel claim-and-keep promotions. |

Information and fields

**Campaign name:** Human-readable internal/public label.

**Game:** Selected game.

**Access type:** Claim now and keep permanently.

**Start and end:** Local date/time and timezone.

**Status:** Draft, Scheduled, Active, Ended, or Cancelled.

**Claim count:** Show only when the available data is complete enough; otherwise label as approximate.

**Public message:** Optional text shown to players.

**Campaign history:** Created, pre-start updates, activation, claims, and cancellation timeline in plain language.

Main actions

Create promotion; Edit scheduled promotion; Cancel promotion; Copy promotion link; Open game page.

Required states and feedback

Draft; Scheduled; Active; Ended; Cancelled; Publishing; Publish failed; Conflicting update.

UX notes

Scheduled terms may be edited only before the effective start. After start, terms are locked and the only campaign action is Cancel. Cancellation blocks later claims but must explicitly state that previous claimants keep their access.

## 5.10 Developer purchase and access activity

| **Status** | **Primary user** | **Purpose**                                                                     |
|------------|------------------|---------------------------------------------------------------------------------|
| New        | Game developers  | Show transactions and free-access grants associated with the developer’s games. |

Information and fields

**Activity rows:** Game, buyer display name when available, Purchase or Promotion claim, amount when paid, date, and status.

**Filters:** Game, Purchases, Promotion claims, Granted, Revoked, Refunded, Disputed, and date.

**Privacy:** Use shortened public identities and explain that activity is derived from marketplace records.

**Completeness notice:** Relay availability may make totals incomplete.

Main actions

Open transaction; Open game; Export later.

Required states and feedback

Loading; Loaded; No activity; Incomplete network data; Verification failed.

## 5.11 Revoke individual game access

| **Status** | **Primary user** | **Purpose** |
|---|---|---|
| New | Game developers | Revoke one durable non-payment access record without changing purchases, campaigns, or other users’ access. |

Information and fields

**Recipient:** Display name when available and shortened account identifier.

**Game:** Title and game ID in optional technical details.

**Access source:** Promotion, Gift, Review copy, Contest prize, or Migration.

**Granted date:** Date the permanent access was issued.

**Current status:** Active or already revoked.

**Consequence:** “This account will lose future download and reinstall access for this grant. Cancelling the original promotion is not required and other claimants are unaffected.”

**Reason:** Optional private/local note unless a future protocol field explicitly supports publishing it.

Main actions

Revoke access; Cancel; Open recipient profile; Open game.

Required states and feedback

Ready; Waiting for signing approval; Publishing revocation; Revoked; Already revoked; Conflicting access history; Network error.

UX notes

Only the developer identity may perform this action. A distribution provider must never be presented as able to revoke user access.

# 6. Account and application settings

## 6.1 Settings home

| **Status** | **Primary user** | **Purpose**                                                                  |
|------------|------------------|------------------------------------------------------------------------------|
| New        | Signed-in users  | Provide one understandable entry point for application and account controls. |

Information and fields

**Sections:** Account, Security, Appearance, Downloads, Network, Data and backup, About.

**Current account:** Name and avatar.

**Application version:** Human-readable version and update state.

Main actions

Open section; Check for updates later.

Required states and feedback

Ready; Update available; Offline.

## 6.2 Security and signing

| **Status**                                     | **Primary user** | **Purpose**                                                             |
|------------------------------------------------|------------------|-------------------------------------------------------------------------|
| Partial across account components; consolidate | Signed-in users  | Manage saved identities, signing connections, and encrypted key backup. |

Information and fields

**Signing connection:** Connected app/service and status.

**Saved identities:** Accounts stored on this device.

**Encrypted key backup:** Export or import an encrypted secret only with explicit warnings.

**Session controls:** Reconnect, disconnect, and remove from device.

**Approval guidance:** Explain when the signing app must be opened.

Main actions

Reconnect; Disconnect; Export encrypted backup; Import encrypted backup; Remove account.

Required states and feedback

Connected; Disconnected; Exporting; Imported; Wrong password; Invalid backup; Removal confirmation.

## 6.3 Network settings

| **Status**                        | **Primary user** | **Purpose**                                                  |
|-----------------------------------|------------------|--------------------------------------------------------------|
| Partial backend support; new page | Advanced users   | Expose connectivity controls without burdening normal users. |

Information and fields

**Connection status:** Simple overall status.

**Connected servers:** Human-readable relay list with latency in advanced mode.

**Automatic discovery:** Recommended on/off setting.

**Custom servers:** Add or remove relay addresses.

**Insecure local connections:** Development-only toggle with explicit warning.

**Reset:** Restore recommended defaults.

Main actions

Reconnect; Add server; Remove server; Restore defaults.

Required states and feedback

Connected; Partially connected; Offline; Testing connection; Invalid address.

UX notes

Keep this out of primary navigation and label it Advanced.

## 6.4 Data and backup

| **Status**                                   | **Primary user** | **Purpose**                                        |
|----------------------------------------------|------------------|----------------------------------------------------|
| Existing component; promote to settings page | Signed-in users  | Back up and restore local application data safely. |

Information and fields

**Backup contents:** Explain profiles, settings, cached marketplace data, purchase records, and what is not included.

**Backup file:** Destination and creation date.

**Restore source:** Selected backup with validation result.

**Conflict handling:** Explain whether restore merges or replaces local data.

**Privacy warning:** Backup may contain sensitive local account information and should be stored securely.

Main actions

Create backup; Restore backup; Choose file; Clear cached data.

Required states and feedback

Creating; Created; Restoring; Restored; Invalid backup; Version incompatible; Storage error.

## 6.5 Appearance

| **Status** | **Primary user** | **Purpose**                 |
|------------|------------------|-----------------------------|
| New        | All users        | Control visual preferences. |

Information and fields

**Theme:** System, Light, or Dark.

**Text size:** Default and larger options.

**Motion:** Reduce animations.

**Content density:** Optional comfortable or compact later.

Main actions

Apply; Restore defaults.

Required states and feedback

Applied.

## 6.6 About and diagnostics

| **Status** | **Primary user**                          | **Purpose**                                                   |
|------------|-------------------------------------------|---------------------------------------------------------------|
| New        | All users; diagnostics for advanced users | Show version, legal information, and exportable support data. |

Information and fields

**Application version:** Version and build.

**Protocol support:** Plain-language compatibility summary; technical versions expandable.

**Licenses:** Open-source notices.

**Diagnostics:** Connection summary, platform, recent non-sensitive errors.

**Privacy:** State what diagnostic export includes.

Main actions

Copy version; Export diagnostics; Open licenses.

Required states and feedback

Ready; Exported; Export failed.

# 7. Cross-screen components and states

**Global top bar:** Current section, back navigation where needed, search entry, account menu, and discreet connectivity indicator.

**Notifications:** Success, warning, and error messages with one clear next action; avoid raw backend messages.

**Confirmation dialogs:** Used for cancelling promotions, revoking an individual user’s durable access, disabling public/timed access, removing accounts, unlisting games, uninstalling files, and replacing releases. Promotion cancellation must state that existing claims remain valid.

**Offline mode:** Show saved marketplace/library data, identify actions that require reconnection, and never imply a network write succeeded before confirmation.

**Signer approval state:** A consistent panel: “Approve this action in your signing app,” with cancel and retry.

**Empty states:** Explain why the page is empty and provide the most relevant action.

**Error details:** Primary message in plain language; expandable technical details with copy action.

**Media fallback:** Broken cover or avatar images must fall back to a designed placeholder without layout collapse.

**Accessibility:** Keyboard navigation, visible focus, semantic headings, accessible dialogs, descriptive button names, sufficient contrast, and reduced-motion support.

**Time and timezone:** Show campaign and release dates in the user’s local timezone and include timezone abbreviation where ambiguity matters.

# 8. Recommended implementation priority

| **Priority**                | **Screens**                                                                                                    | **Reason**                                                        | **Exit condition**                                                                                                 |
|-----------------------------|----------------------------------------------------------------------------------------------------------------|-------------------------------------------------------------------|--------------------------------------------------------------------------------------------------------------------|
| P0 - Core player journey    | Store home, Browse, Game details, Payment dialog, Promotion claim confirmation, Public/timed access notice, Success, Library, Install progress | Completes discovery-to-play across all four acquisition paths. | A user can distinguish purchased ownership, permanent promotion claims, and temporary/public access without protocol terminology. |
| P0 - Core developer journey | Developer dashboard, Create game steps, Provider selection, Review and publish, Manage game, Promotions | Makes the new access and distribution model usable by developers. | A developer can configure paid sales, explicit public access, timed access, and separate claim-and-keep promotions with clear durability rules. |
| P1 - Trust and recovery     | Purchase receipt, Permanent access record, Acquisition history, Security and signing, Data and backup, Network settings                             | Improves ownership clarity and failure recovery.                  | Users can inspect access records, restore local state, and diagnose connection problems.                           |
| P1 - Release management     | Releases, Installed game management, Install locations                                                         | Supports updates and long-term library use.                       | Developers can publish updates and players can update, verify, and remove local files safely.                      |
| P2 - Community and polish   | Search page, Collections, Social, Achievements refinements, Appearance, About                                  | Improves discovery and retention after core flows are stable.     | Community and personalization do not compromise core marketplace reliability.                                      |

# 9. User-facing terminology

| **Avoid in primary UI** | **Use instead**                | **Where technical term may appear** |
|-------------------------|--------------------------------|-------------------------------------|
| NIP-46 / NIP-07         | Signing app / browser signer   | Advanced connection details         |
| npub / pubkey           | Account ID / public identifier | Profile technical details           |
| relay                   | Network server                 | Advanced network settings           |
| kind 30402 listing      | Game page / game listing       | Technical details                   |
| kind 1020 receipt       | Purchase record / receipt      | Receipt technical details           |
| entitlement grant       | Permanent game access record   | Access-record technical details     |
| campaign event          | Claim-and-keep promotion       | Promotion technical details         |
| fulfillment key         | Distribution authorization     | Provider technical details          |
| ADP server              | Distribution provider          | Provider technical details          |
| file hash               | File verification              | Release technical details           |
| game coordinate         | Game ID                        | Technical details                   |
| NIP-98 authorization    | Secure request approval        | Diagnostics only                    |

# 10. Basis and scope

This inventory is based on the current Arcadestr UI/component inventory, NIP-102 paid purchase records, the Entitlement Grant draft, the ADP free-acquisition amendment, and ADP distribution/provisioning flows. The ADP server itself is a backend service and does not require a general end-user administration UI for the Arcadestr application. Operator administration is outside this document unless Arcadestr later ships a hosted-provider console.
