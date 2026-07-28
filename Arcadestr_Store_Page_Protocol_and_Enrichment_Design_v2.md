# Arcadestr Store Page Protocol and Marketplace Enrichment Design

**Status:** Proposed experimental design, revision 2  
**Scope:** Arcadestr presentation metadata, Store/Browse enrichment, Game Detail rendering, and Publisher Studio editing

> **Core decision:** Use a separate parameterized replaceable Nostr event for presentation metadata. The existing `kind:30402` listing remains authoritative for commerce, access, distribution, builds, compatibility, and ownership.

## 1. Executive summary

Arcadestr already supports decentralized game publication, Lightning purchases, entitlement claims, ADP distribution, authenticated downloads, ownership history, installation, platform filtering, campaign management, and detailed game views. What it lacks is a dedicated publisher-controlled presentation layer capable of producing polished game pages and visually distinctive marketplace cards.

This design introduces an Arcadestr Store Page event that can enrich one or more `kind:30402` listings owned by the same publisher. The Store Page contains presentation and discovery metadata such as capsule artwork, hero media, screenshots, trailers, descriptions, feature sections, genres, languages, accessibility data, system requirements, release information, and external links.

The design preserves five load-bearing properties:

1. `kind:30402` remains authoritative for all commerce, access, compatibility, fulfillment, and distribution decisions.
2. A Store Page may enrich multiple platform-specific or edition-specific listings from the same publisher.
3. Applying Store Page enrichment requires a validated two-way association between the listing and Store Page.
4. Missing, invalid, stale, unsupported, or unsafe Store Page data always degrades to the listing.
5. Publisher content is never rendered without strict Markdown, URL, media, and size validation.

## 2. Goals and non-goals

### 2.1 Goals

- Give developers a structured editor for polished game pages.
- Support a shared presentation page across Windows, Linux, macOS, architecture-specific, or edition-specific listings.
- Provide typed hero, capsule, thumbnail, screenshot, trailer, and feature media.
- Support sanitized Markdown, constrained feature layouts, languages, accessibility, and platform requirements.
- Enrich Store and Browse cards without blocking initial listing rendering.
- Keep presentation publishing independent from build, price, access, campaign, and fulfillment changes.
- Reuse Arcadestr's replacement ordering, optimistic concurrency, cache, URL validation, stale-response protection, and account isolation patterns.
- Provide a live preview using the same presentation components as the buyer-facing page.

### 2.2 Non-goals for version 1

- Replacing NIP-99 or changing ADP purchase/download semantics.
- Making Store Page data authoritative for compatibility or install availability.
- Defining a complete editions, DLC, bundles, or SKU protocol.
- Hosting, transcoding, or moderating publisher media.
- Adding reviews, ratings, recommendations, or community features.
- Collaborative multi-author editing.
- Full localization of the complete Store Page document.
- Relay-side filtering by non-standard multi-character tags.

## 3. Authority boundaries

| Resource | Authoritative responsibility |
|---|---|
| `kind:30402` listing | Price, currency, `lud16`, acquisition policy, campaign pointers, install platforms, ADP servers, file hash, version, fulfillment authorization, and listing validity |
| Store Page event | Presentation, media, descriptive metadata, requirements, languages, accessibility, and external links |
| Campaign chain | Claim campaign terms and lifecycle |
| NIP-102 receipt / Entitlement Grant | Durable paid or non-monetary ownership |
| Current listing + ADP server | Distribution authorization, current artifact metadata, integrity, and download execution |

A Store Page **must never override or synthesize**:

- price or currency;
- `lud16` or payment routing;
- public, timed, gated, or claim access policy;
- campaign validity or state;
- supported install platforms;
- ADP server authorization;
- file hashes, builds, versions, or download URLs;
- fulfillment keys or authorization chains;
- ownership, receipts, grants, or installed state.

System requirements in a Store Page are descriptive only. A client must not infer that a build exists for a platform unless the authoritative listing declares that platform.

## 4. Gate 0: event-kind safety

No implementation work should begin until the experimental event kind is selected and checked for collisions.

### 4.1 Required gate

1. Inventory currently assigned Nostr kinds from the authoritative allocation source used by the project.
2. Inventory all provisional kinds already used by Arcadestr and ADP.
3. Select one experimental parameterized-replaceable kind.
4. Centralize it in a single protocol constants module.
5. Never embed the numeric value directly in UI, SQL, filters, parser branches, or tests outside that module.
6. Advertise it as provisional in documentation and server/client capability reporting.
7. Add regression tests asserting it differs from:
   - `30402`;
   - Entitlement Grant provisional kind `1030`;
   - ADP campaign provisional kind `1031`;
   - every known assigned or project-reserved kind recorded in the repository.

The selected number must not be presented as an interoperable production allocation.

## 5. Event model

### 5.1 Event type

Use a parameterized replaceable event:

```text
kind: <experimental-store-page-kind>
pubkey: <publisher pubkey>
d: <game-presentation-id>
```

The `d` tag identifies a **game presentation**, not an individual listing. One Store Page may reference multiple listings owned by the same publisher.

### 5.2 Required association tags

A Store Page must include:

- exactly one `d` tag;
- one or more `a` tags;
- every `a` tag must contain a complete `30402:<publisher-pubkey>:<listing-d>` coordinate;
- every referenced coordinate must encode the Store Page author's pubkey;
- duplicate `a` coordinates are invalid.

Example:

```jsonc
{
  "kind": "<experimental-store-page-kind>",
  "pubkey": "<developer-pubkey>",
  "created_at": 1780000000,
  "tags": [
    ["d", "example-game"],
    ["a", "30402:<developer-pubkey>:example-game-linux"],
    ["a", "30402:<developer-pubkey>:example-game-windows"],
    ["title", "Example Game"],
    ["summary", "A concise description for cards and headers."],
    ["developer", "Example Studio"],
    ["publisher", "Example Studio"],
    ["release_date", "2026-10-12"],
    ["genre", "action"],
    ["genre", "adventure"],
    ["feature", "single-player"],
    ["feature", "controller-support"],
    ["language", "en", "interface,audio,subtitles"],
    ["language", "pt-BR", "interface,subtitles"]
  ],
  "content": "{...versioned JSON...}",
  "sig": "<developer-signature>"
}
```

### 5.3 Listing pointer

Each participating `kind:30402` listing should contain an advisory pointer:

```jsonc
["store_page", "30407:<publisher-pubkey>:<game-presentation-id>", "wss://relay.example.com"]
```

The relay hint is optional and must be a structurally valid public `wss` URL with a host and no
embedded credentials. Insecure `ws` and literal local, private, link-local, unspecified, or multicast
hosts are rejected before network access. An admitted hint is added to Arcadestr's normal relay strategy as advisory coverage and is
never queried as the exclusive source of truth.

The pointer is a discovery aid only. It does not make Store Page data authoritative and does not prove association by itself.

Version 1 activates a pointer only when the signed listing has exactly one `store_page` tag and that
tag is valid. Duplicate valid pointers, conflicting valid pointers, and a valid pointer accompanied
by any malformed pointer produce typed diagnostics and no active association. Malformed pointer
tags do not invalidate the listing and remain unmanaged tags during unrelated listing replacements.

### 5.4 Two-way association rule

Store Page enrichment may be applied only when all of the following are true:

```text
listing has a store_page pointer to the Store Page coordinate
AND Store Page contains the listing coordinate in an a tag
AND listing.pubkey == store_page.pubkey
AND both signatures and coordinates are valid
```

This two-way rule prevents:

- an unrelated Store Page from claiming another publisher's listing;
- a stale or malformed listing pointer from attaching an unrelated page;
- accidental publisher mismatches;
- enrichment based solely on one mutable side of the relationship.

The reciprocal check is always performed against the currently loaded signed listing event and the
currently selected valid Store Page replacement. Cached association state is not authority. A
listing pointer change immediately detaches the previously cached page, and a valid Store Page
replacement that removes the listing's `a` tag also detaches it. Neither case affects listing use.

### 5.5 Multiple listings and platform publication

A single game may use separate listings for Windows, Linux, macOS, architectures, regional offers, or separately distributed artifacts. These listings may share one Store Page when they present the same game.

Example:

```text
Store Page:  <store-kind>:publisher:example-game
Listings:
- 30402:publisher:example-game-linux
- 30402:publisher:example-game-windows
- 30402:publisher:example-game-macos
```

Every listing remains independently authoritative for its own commerce, platform, build, server, and fulfillment fields.

## 6. Tags and content schema

### 6.1 Tag policy

Tags provide compact mirrors for lightweight clients and local indexing after retrieval. Except for standard single-letter tags, clients must not assume generic relay indexing.

| Tag | Cardinality | Purpose |
|---|---:|---|
| `d` | exactly 1 | Game presentation identifier |
| `a` | 1 or more | Associated `kind:30402` listing coordinates |
| `title` | 0 or 1 | Compact presentation title |
| `summary` | 0 or 1 | Compact card/header summary |
| `developer` | 0 or 1 | Human-readable developer name |
| `publisher` | 0 or 1 | Human-readable publisher name |
| `release_date` | 0 or 1 | ISO 8601 date where known |
| `genre` | repeatable | Normalized genre identifier |
| `feature` | repeatable | Modes or game features |
| `language` | repeatable | BCP 47 code plus capabilities |
| `content_rating` | repeatable | Rating value and system |
| `website` | 0 or 1 | Compact website mirror |
| `support` | 0 or 1 | Compact support mirror |

Standard discovery should rely on:

- event kind;
- author;
- `#d`;
- `#a`;
- event ID.

Genre, feature, language, developer, publisher, and rating filtering should occur against Arcadestr's local parsed cache. Relay-side filtering is optional only on relays that explicitly document non-standard generic tag indexing.

### 6.2 Versioned content envelope

```jsonc
{
  "schema": "io.arcadestr.store-page",
  "version": 1,
  "basic": {
    "title": "Example Game",
    "summary": "A concise description.",
    "developer": "Example Studio",
    "publisher": "Example Studio",
    "release_date": "2026-10-12"
  },
  "description_markdown": "## About this game\n...",
  "discovery": {
    "genres": ["action", "adventure"],
    "features": ["single-player", "controller-support"]
  },
  "media": [],
  "sections": [],
  "languages": [],
  "requirements": {},
  "accessibility": [],
  "links": {}
}
```

Rules:

- Unknown fields must be ignored.
- Unsupported schema versions must not hide or invalidate the listing.
- Supported JSON content is authoritative over duplicated compact tags.
- Conflicting duplicated presentation values should produce parser diagnostics, not invalidate the complete event.
- Identity and association fields remain tag-authoritative and cannot be overridden by JSON content.

### 6.3 Explicit field precedence

The parser must normalize tags and content into one model using the following order:

| Field | Resolution order |
|---|---|
| Title | `content.basic.title` → `title` tag → listing title → `Untitled game` |
| Summary | `content.basic.summary` → `summary` tag → first sanitized description paragraph → truncated listing description |
| Developer | `content.basic.developer` → `developer` tag → publisher profile fallback |
| Publisher | `content.basic.publisher` → `publisher` tag → publisher profile fallback |
| Release date | `content.basic.release_date` → `release_date` tag |
| Genres | `content.discovery.genres` → `genre` tags → compatible listing tags |
| Features | `content.discovery.features` → `feature` tags → compatible listing tags |
| Languages | `content.languages` → `language` tags |
| Website | `content.links.website` → `website` tag |
| Support | `content.links.support` → `support` tag |

The parser contract, tests, and normalized model should encode these rules directly rather than relying on UI code to infer them.

## 7. Media model

Media order is the order of the array. A separate numeric order property is unnecessary.

```jsonc
{
  "media": [
    {
      "id": "hero-main",
      "type": "image",
      "role": "hero",
      "url": "https://cdn.example.com/hero.webp",
      "alt": "The protagonist overlooking a ruined city",
      "width": 1920,
      "height": 620
    },
    {
      "id": "capsule-main",
      "type": "image",
      "role": "capsule",
      "url": "https://cdn.example.com/capsule.webp",
      "alt": "Example Game cover"
    },
    {
      "id": "trailer-main",
      "type": "video",
      "role": "trailer",
      "url": "https://cdn.example.com/trailer.webm",
      "thumbnail_url": "https://cdn.example.com/trailer.webp",
      "caption": "Gameplay trailer"
    },
    {
      "id": "screenshot-1",
      "type": "image",
      "role": "screenshot",
      "url": "https://cdn.example.com/screenshot-1.webp",
      "alt": "Combat against a mechanical guardian",
      "caption": "Real-time combat"
    }
  ]
}
```

Version 1 roles:

- `hero`;
- `capsule`;
- `thumbnail`;
- `screenshot`;
- `trailer`;
- `feature`.

Validation:

- Media IDs must be unique.
- Only one `hero`, `capsule`, and `thumbnail` may be active.
- Unsupported roles may be retained but rendered generically or ignored.
- Missing alt text should produce an editor warning for images used in visible UI.
- Video must use Arcadestr-owned rendering components; arbitrary embed HTML is forbidden.

## 8. Rich sections

### 8.1 Feature sections

```jsonc
{
  "sections": [
    {
      "id": "exploration",
      "heading": "Explore a forgotten world",
      "body_markdown": "Travel through interconnected regions...",
      "media_id": "feature-exploration",
      "layout": "media-left"
    }
  ]
}
```

Supported layouts:

- `text`;
- `media-left`;
- `media-right`;
- `media-wide`.

Unknown layouts should fall back to a safe vertical layout. Raw HTML is never allowed.

### 8.2 System requirements

```jsonc
{
  "requirements": {
    "linux-x86_64": {
      "minimum": {
        "os": "Ubuntu 22.04 or equivalent",
        "processor": "AMD Ryzen 3 1200 or equivalent",
        "memory": "8 GB RAM",
        "graphics": "Vulkan-capable GPU with 4 GB VRAM",
        "storage": "20 GB available space",
        "additional": "64-bit operating system required"
      },
      "recommended": {
        "os": "Ubuntu 24.04 or equivalent",
        "processor": "AMD Ryzen 5 3600 or equivalent",
        "memory": "16 GB RAM",
        "graphics": "Vulkan-capable GPU with 8 GB VRAM",
        "storage": "20 GB SSD space"
      }
    }
  }
}
```

Platform keys should use the same normalized platform identifiers used by Arcadestr listings. Requirements may be displayed only for platforms represented by the associated authoritative listings currently being presented.

### 8.3 Languages

```jsonc
{
  "languages": [
    {
      "code": "en",
      "interface": true,
      "audio": true,
      "subtitles": true
    },
    {
      "code": "pt-BR",
      "interface": true,
      "audio": false,
      "subtitles": true
    }
  ]
}
```

Language identifiers should use BCP 47 where practical.

### 8.4 Accessibility

```jsonc
{
  "accessibility": [
    { "feature": "subtitles", "supported": true },
    { "feature": "subtitle-size", "supported": true },
    {
      "feature": "colorblind-modes",
      "supported": true,
      "notes": "Protanopia, deuteranopia, and tritanopia presets"
    },
    { "feature": "screen-reader", "supported": false }
  ]
}
```

Initial vocabulary:

- `subtitles`;
- `subtitle-size`;
- `closed-captions`;
- `colorblind-modes`;
- `high-contrast`;
- `text-scaling`;
- `input-remapping`;
- `difficulty-options`;
- `camera-shake-control`;
- `screen-reader`.

Unknown values should be retained and displayed generically. An explicit `false` value is useful information and should not be silently removed.

### 8.5 Links

```jsonc
{
  "links": {
    "website": "https://example.com",
    "support": "https://example.com/support",
    "documentation": "https://docs.example.com",
    "source": "https://github.com/example/game",
    "community": "https://example.com/community",
    "privacy_policy": "https://example.com/privacy"
  }
}
```

JSON link values are authoritative for Arcadestr rendering. Compact `website` and `support` tags are fallback mirrors.

## 9. Phase 0 security gate

Store Page parsing and rendering must not be implemented until the sanitization and URL policy is defined, shared by desktop and web targets, and covered by adversarial tests.

### 9.1 Markdown contract

- Shared core code parses CommonMark with `pulldown-cmark` using no optional extensions. Raw block
  and inline HTML events are removed before deterministic HTML generation. Consequently scripts,
  event-handler attributes, iframes, objects, embeds, and publisher-supplied styles cannot reach the
  sanitized output.
- Markdown links and images pass through the same structural HTTPS URL validator as structured
  fields. Unsafe link/image wrappers are removed while their text or alt text is retained, and a
  typed diagnostic is emitted.
- Source and sanitized-output sizes are bounded. Nesting is limited to 12 levels, headings to level
  3, links to 64, and images to 16. Exceeding a complexity limit rejects the Store Page event.
- Sanitized output does not add `target="_blank"`; therefore it creates no opener relationship.
  Any future renderer that adds a new browsing context must also add `rel="noopener noreferrer"`.
- Raw `content` remains available only for diagnostics and forward compatibility. Rendering must
  consume the parser's `SanitizedStorePageContent` and `SanitizedMarkdown` values.

### 9.2 URL contract

- Parse with the shared `url` crate's WHATWG parser; prefix checks are not used as validation.
- Production Store Page content allows only absolute `https` URLs with a non-empty parsed host.
  Arcadestr currently has no safely scoped development-only exception, so `http` is rejected in all
  Store Page events.
- Reject `javascript`, `data`, `file`, `blob`, `tauri`, `asset`, every custom scheme, embedded
  usernames/passwords, surrounding whitespace, malformed or ambiguous authority forms, and URLs
  over 2,048 bytes.
- Store canonical parser output after validation. Never interpret publisher URL values as local
  filesystem paths and do not fetch media during parsing.
- This policy applies to media, thumbnails, all structured external links, compact website/support
  mirrors, and Markdown links/images.

### 9.3 Video contract

- Version 1 permits direct HTTPS `.mp4` and `.webm` video URLs only. Extension matching is
  case-insensitive and uses the parsed URL path, excluding query strings and fragments.
- HLS/DASH playlists, MOV and other containers, provider pages, iframe/embed HTML, JavaScript
  players, and remote scripts are unsupported. Unsupported video media items are omitted with a
  typed diagnostic.
- Posters/thumbnails pass through the normal HTTPS URL policy. An invalid optional thumbnail is
  omitted without removing an otherwise valid media item.
- No media is fetched in the protocol layer. Future renderers must disable autoplay by default and
  must not infer a playable format from remote response content alone.

### 9.4 Current Tauri posture

Because the current application documentation notes `withGlobalTauri: true` and `csp: null`, publisher-controlled content must be treated as a high-risk untrusted input surface. Sanitization cannot depend on CSP alone. A strict CSP remains separately recommended and should be reviewed before release.

## 10. Validation rules

### 10.1 Event-level validation

1. Verify the event signature.
2. Require the configured experimental Store Page kind.
3. Require exactly one non-empty `d` tag.
4. Require at least one `a` tag.
5. Parse every `a` as a `kind:30402` coordinate.
6. Require every coordinate's publisher to equal `event.pubkey`.
7. Reject duplicate associated listing coordinates.
8. Parse supported content schema/version.
9. Enforce field, collection, URL, Markdown, and complete-event limits.
10. Normalize duplicated tag/content fields using the explicit precedence contract.
11. Apply parameterized-replaceable ordering through Arcadestr's centralized resolver.

### 10.2 Listing association validation

Before enriching a listing:

1. Verify the listing event and coordinate.
2. Parse its `store_page` pointer.
3. Require the pointer author to equal the listing publisher.
4. Fetch or load the referenced Store Page.
5. Require Store Page author to equal listing publisher.
6. Require the Store Page to contain the exact listing coordinate in an `a` tag.
7. Apply only sanitized normalized presentation fields.

### 10.3 Failure behavior

- Wrong kind, invalid signature, identity/association failures, malformed JSON, unsupported schema
  or version, duplicate IDs, and any event, field, collection, or Markdown complexity limit failure
  reject the complete Store Page event. Invalid Store Pages are ignored.
- Invalid or stale pointers are ignored.
- The listing remains visible and usable.
- A media item with an invalid URL, unsupported type, unsupported direct-video format, or duplicate
  singleton role is omitted with a typed diagnostic. An invalid optional thumbnail is removed while
  retaining the media item.
- Invalid structured or compact external links are omitted with typed diagnostics. Content remains
  authoritative over compact mirrors: an invalid content link does not fall back to its tag mirror.
- Raw HTML and unsafe Markdown link/image wrappers are stripped with typed diagnostics. Exceeding a
  Markdown size, output, depth, heading, link-count, or image-count limit rejects the event.
- A bad section media reference renders the sanitized section without media.
- Unsupported schema versions use listing fallbacks.
- Relay incompleteness is not interpreted as deletion.
- Store Page failure never changes purchase, claim, install, ownership, or compatibility behavior.

## 11. Discovery and replacement resolution

### 11.1 Preferred pointer lookup

For a listing with a valid pointer, fetch the Store Page by addressable coordinate or by author plus `#d`:

```jsonc
{
  "kinds": ["<store-page-kind>"],
  "authors": ["<publisher-pubkey>"],
  "#d": ["<game-presentation-id>"]
}
```

Then validate the two-way association.

Before querying, Arcadestr loads only the cache row for the exact pointer coordinate and revalidates
its raw signed event. Cached and relay candidates then compete in one centralized replaceable-event
ordering operation. This prevents an older relay response from replacing or detaching a newer cache
entry. A valid result obtained from incomplete coverage may be used but remains marked as having an
unavailable refresh.

### 11.2 Coordinate recovery lookup

When the pointer is absent, stale, or unresolved, clients may query by listing coordinate:

```jsonc
{
  "kinds": ["<store-page-kind>"],
  "#a": ["30402:<publisher-pubkey>:<listing-d>"]
}
```

Any result still requires author and association validation. Coordinate recovery is a fallback, not proof that the listing intended to use the page.

Recovery results are grouped by full Store Page coordinate and replacement ordering is applied
independently per group. Every event is locally checked for author, `d`, and exact listing `a` value;
clients never infer author/coordinate tuple pairing from batched relay filters. Recovery may locate
and cache a page when no pointer exists, but it cannot attach that page. When a pointer exists,
recovery attaches only after complete preferred-coordinate resolution and a successful reciprocal
check. Incomplete coverage is reported as unavailable evidence rather than absence.

### 11.3 Batch lookup for visible cards

Marketplace enrichment should target visible cards, not the complete result set.

```text
Filtered marketplace: 600 listings
Visible window: 50 listings
Store Page targets: referenced pages for those 50 listings
```

Deduplicate Store Page coordinates before querying because multiple visible platform listings may point to the same page.

Group by publisher where practical:

```jsonc
{
  "kinds": ["<store-page-kind>"],
  "authors": ["<publisher-a>"],
  "#d": ["game-one", "game-two", "game-three"]
}
```

Validate every returned `(author, d)` against the requested set.

### 11.4 Replacement ordering

- Higher `created_at` wins.
- Equal timestamps use Arcadestr's existing deterministic event-ID tie-break.
- An invalid newer event must not replace the newest valid cached event.
- Do not introduce a Store Page-specific resolver.
- Cache and relay candidates for one coordinate must be passed through the same centralized resolver.
- A valid newer non-reciprocal replacement supersedes the cached event and detaches presentation;
  an invalid or unsupported newer candidate does neither.

## 12. Marketplace enrichment flow

### 12.1 Progressive loading

```text
1. Load cached kind:30402 listings.
2. Render listing-based cards immediately.
3. Filter, sort, and paginate.
4. Parse Store Page pointers for visible listings.
5. Deduplicate referenced Store Page coordinates.
6. Read valid Store Pages from local cache.
7. Validate two-way associations and apply cached enrichment.
8. Batch-fetch fresh Store Pages.
9. Resolve valid replacements.
10. Persist fresh Store Pages and association data.
11. Update visible cards incrementally.
```

Presentation enrichment must never cause a full-page loading spinner. Cards should appear from listing data first and improve when cached or fresh presentation data arrives.

Gate 3 uses a batch desktop command rather than placing signed events in UI models. Each visible
listing is represented by its canonical `30402:<publisher-hex>:<d>` coordinate and the exact event
ID currently displayed. The backend performs
one batched kind-30402 query, applies centralized replacement ordering, and retains the exact signed
event only when its ID matches the request, then uses it for reciprocal validation. The backend also
persists that validated raw listing event in a Store Page-specific association cache. On temporary
relay failure, the exact cached listing ID may be reparsed and combined with the cached Store Page to
validate reciprocity offline; a different requested listing ID never reuses it. Publisher writes still
require a successful relay refresh, with cached and relay candidates competing under centralized
replacement ordering. The backend never reconstructs listing events and never sends raw signed
listing or Store Page JSON to the UI.

The command groups Store Page pointer lookups by publisher and expected `d` values, then performs a
single batched `#a` recovery query. Duplicate listing coordinates are deduplicated before relay work,
the backend caps requests at 64 listings, and every cross-product relay result is checked locally.
Ordinary relay success is not treated as authoritative proof of absence; missing evidence remains
`Unavailable` unless coverage is explicitly complete. The response separates cached and refreshed
updates and echoes the caller generation. Standalone web builds return an explicit unavailable error.

The app owns a presentation-only map keyed by canonical listing coordinate. Existing cards render
from listings immediately; cached updates are applied before refreshed updates. A changed listing
event ID invalidates that coordinate's previous presentation, stale generations are ignored, and an
`Unavailable` refresh preserves the last valid presentation. Confirmed invalid, absent, or
non-reciprocal results detach it.

### 12.2 Card enrichment fields

| Field | Preferred source | Fallback |
|---|---|---|
| Title | normalized Store Page title | listing title → `Untitled game` |
| Summary | normalized Store Page summary | first sanitized paragraph → truncated listing description |
| Capsule | Store Page capsule | hero → first valid listing image → placeholder |
| Genres | normalized Store Page genres | compatible listing tags |
| Features | normalized Store Page features | compatible listing tags |
| Release date | Store Page release date | hidden |
| Developer/publisher | Store Page values | publisher profile or npub |

Version 1 cards should initially use only capsule, title, summary, genres, features, and release date. Rich sections, requirements, accessibility, and the full media set belong on Game Detail.

The card model contains only Store Page coordinate/event ID, optional title and summary, optional
capsule and hero URLs, genres, features, and release date. It is never merged into `GameListing`.
Display resolution is:

- title: Store Page title, then listing title;
- summary: Store Page summary, then a bounded listing-description excerpt;
- image: Store Page capsule, Store Page hero, first valid listing image, existing placeholder;
- badges: at most two Store Page genres/features, then existing listing categories.

Price, acquisition, campaign, ownership, install action, compatibility, version, and fulfillment
continue to use only listing/account/device state. Store Front may use valid hero/capsule media for
the already listing-selected featured game; Store Page data never selects eligibility.

### 12.3 Filtering and sorting

Local filtering may use Store Page metadata for:

- genre;
- theme;
- player mode;
- feature support;
- language;
- accessibility;
- release date;
- developer/publisher name.

Gate 3 does not expose Store Page-dependent filter controls. Existing search, category, price,
access, campaign, and platform filters continue to operate on listing data while enrichment is
pending. Presentation-dependent genre/feature/release filters are therefore deterministically
disabled until a later phase can expose confirmed-only filtering without removing unknown pending
results.

The following remain listing, campaign, credential, or device-derived:

- platform compatibility;
- price;
- gated/public/timed access;
- active campaigns;
- ownership;
- installed state;
- current version;
- download support.

## 13. Game Detail flow

### 13.1 Immediate render

When a card is opened, pass the current listing and optional valid cached Store Page into `GameDetailView` so it can render without waiting for network refreshes.

Gate 4 keeps `GameDetailPresentation` separate from `GameDetailCommerce`. Presentation contains only
normalized descriptive fields, construction-controlled sanitized HTML, validated media, feature
sections, listing-gated requirements, languages, publisher-claimed accessibility, and safe HTTPS
links. Commerce is always rebuilt from the selected listing coordinate, price, acquisition policy,
campaigns, ownership/install state, listing platforms, version, server availability, and file hash.

### 13.2 Independent refreshes

Refresh independently:

- the current `kind:30402` listing;
- the referenced Store Page;
- durable ownership;
- campaign state;
- publisher profile;
- installed state.

Presentation refresh failure must not block commerce or installation state. A listing refresh failure must not make Store Page data authoritative for commerce.

Detail requests bind the canonical listing coordinate, exact displayed listing event ID, active
account reaction, and a detail-local generation. Cached presentation is applied before refreshed
presentation in the response. `Unavailable` preserves cached detail; invalid, unsupported,
non-reciprocal, or pointer-changed results detach it. Navigation cleanup increments the generation,
so late results cannot update another detail page.

Native commerce actions remain disabled until the detail command confirms that the exact displayed
listing event ID is still current. The backend first selects the newest valid replacement for the
coordinate and only then compares its ID with the displayed event. Store Page availability never
serves as that confirmation.

Marketplace cache persistence retains the signed listing's exact price amount and currency. The
derived `price_sats` compatibility field is populated only for `SAT`/`SATS`; cache reload must not
reinterpret another currency as sats.

### 13.3 Separate view models

```rust
pub struct GamePresentation {
    pub store_page_coordinate: Option<String>,
    pub associated_listing_coordinates: Vec<String>,
    pub title: String,
    pub summary: String,
    pub description_markdown: String,
    pub hero: Option<MediaItem>,
    pub capsule: Option<MediaItem>,
    pub screenshots: Vec<MediaItem>,
    pub trailers: Vec<MediaItem>,
    pub sections: Vec<StoreSection>,
    pub genres: Vec<String>,
    pub features: Vec<String>,
    pub languages: Vec<LanguageSupport>,
    pub requirements: PlatformRequirements,
    pub accessibility: Vec<AccessibilityFeature>,
    pub links: StoreLinks,
}

pub struct GameCommerceState {
    pub listing_coordinate: String,
    pub price: Option<GamePrice>,
    pub acquisition: AcquisitionPolicy,
    pub platforms: Vec<String>,
    pub campaign: Option<DiscoveredCampaign>,
    pub ownership: OwnershipState,
    pub install_state: InstallState,
    pub servers: Vec<String>,
    pub version: Option<String>,
}
```

Keeping these models separate prevents presentation data from influencing buy, claim, compatibility, or install behavior.

### 13.4 Safe rich rendering

- The native command returns sanitized HTML strings only inside a private bridge DTO. The public app
  `SafeStorePageHtml` has no public string constructor and does not implement deserialization.
- One reviewed component is the only Store Page `inner_html` sink. Raw Markdown and raw event content
  are never IPC fields.
- The media viewer supports validated hero/screenshot images and direct MP4/WebM trailers with
  browser/WebView controls, metadata-only preload, no autoplay, bounded escaped alt/caption text,
  keyboard-selectable thumbnails, and a native dialog for expanded media with Escape/focus handling.
- Sections render only `text`, `media-left`, `media-right`, or `media-wide`; missing media references
  remain text-only.
- Requirements are emitted only for platform tags on the currently opened signed listing. Store
  Page platform-like fields never imply compatibility.
- Language capabilities are informational and never change app localization. Accessibility entries
  are labeled as publisher-provided claims.
- External links are already HTTPS-policy validated and open in a separate browsing context with
  `noopener noreferrer`; no custom/Tauri schemes are accepted.

### 13.5 Detail fallbacks

- title: Store Page title, then listing title;
- summary: Store Page summary, then bounded listing description;
- hero: Store Page hero, capsule, listing image, placeholder;
- full description: sanitized Store Page HTML, then escaped listing description;
- genres/features: normalized Store Page values, then listing categories.

The acquisition panel continues to use the listing title and coordinate internally even when the
display title differs.

## 14. Cache and storage design

Use a dedicated Store Page cache rather than embedding presentation into marketplace listing rows.
The current listing is always consulted for reciprocal association, so attachment state is not
persisted as authority.

```sql
CREATE TABLE store_pages (
    store_page_coordinate TEXT PRIMARY KEY,
    event_id TEXT NOT NULL,
    publisher_pubkey TEXT NOT NULL,
    d_tag TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    raw_event_json TEXT NOT NULL,
    schema_version INTEGER NOT NULL,
    sanitizer_policy_version INTEGER NOT NULL,
    sanitized_content_json TEXT NOT NULL,
    diagnostics_json TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX idx_store_pages_publisher
    ON store_pages(publisher_pubkey);

```

Persistence rules:

- Cache only valid signed events with a supported normalized representation.
- Preserve raw event JSON for forward compatibility and diagnostics.
- Persist sanitized content and typed diagnostics separately from raw wire content. Never
  deserialize raw event content directly into a renderable type.
- Store the sanitizer/content-policy version. Every cache load reparses the signed raw event; an old
  policy version triggers sanitization recomputation and an atomic rewrite only if the same event ID
  is still cached. If recomputation fails, return a stale-policy state and continue relay discovery.
- Do not delete a Store Page because one listing disappears temporarily from relay results.
- Do not apply enrichment unless the current listing pointer and Store Page `a` reference both validate.
- Invalid newer events must not replace valid cached state.
- Equal timestamps use the centralized lower-event-ID tie-break in both memory and the atomic SQL
  conflict guard. Relay absence never deletes or downgrades a cache row.

## 15. Publisher Studio workflow

### 15.1 Game management navigation

```text
Publisher Studio
└── Game Management
    ├── Store Page
    │   ├── Basic Information
    │   ├── Associated Listings
    │   ├── Media
    │   ├── About the Game
    │   ├── Feature Sections
    │   ├── Languages
    │   ├── System Requirements
    │   ├── Accessibility
    │   ├── Links
    │   └── Preview
    ├── Publication
    ├── Build and Distribution
    ├── Pricing and Access
    └── Campaigns
```

### 15.2 Association choices

When publishing or replacing a listing, the publisher may:

- link it to an existing Store Page;
- create a new Store Page;
- leave it without a Store Page;
- clone an existing Store Page into a new presentation ID.

When a new listing coordinate is introduced, the editor should present:

```text
Choose presentation association
├── Link existing Store Page
├── Clone existing Store Page
└── Create new Store Page
```

### 15.3 Major versions and changed d-tags

Changing a listing `d` creates a new listing coordinate. The old Store Page is not automatically copied or reassigned.

The editor should offer:

- **Link existing page:** use when the new coordinate represents another platform/build of the same game presentation.
- **Clone forward:** use when a separately purchased major version needs independent marketing content.
- **Create new:** start with an empty page.

Cloning must:

- generate a new Store Page `d`;
- copy presentation fields and media references;
- clear all listing associations;
- require explicit selection of new associated listings;
- avoid copying optimistic-concurrency event IDs.

### 15.4 Editing and publishing

```text
1. Open Game Management.
2. Load selected listings and their current Store Page association.
3. Load the newest valid Store Page event.
4. Retain the loaded event ID for optimistic concurrency.
5. Edit a local draft.
6. Validate fields, media, Markdown, URLs, limits, and associations.
7. Preview through the buyer-facing presentation components.
8. Recheck for a newer remote Store Page event.
9. Sign and publish the Store Page replacement.
10. Update listing pointers through the normal kind:30402 replacement flow.
11. Confirm propagation independently for Store Page and listing updates.
12. Update the local cache and association records.
```

Because Store Page publication and listing-pointer updates are separate Nostr events, the operation is not atomic. The UI must expose partial success and offer repair actions:

- Store Page published, pointer update failed;
- listing pointer updated, Store Page propagation not confirmed;
- only some selected listings updated;
- stale pointer remains on one platform listing.

Association validation prevents partial failures from becoming a security issue; they only delay enrichment.

### 15.5 Editor structure

Publisher Studio exposes Store Page management from the selected published game's management view.
The editor is separate from Network publication so presentation edits cannot alter listing-derived
price, currency, acquisition, platforms, builds, fulfillment, ownership, or installation state.

The editor provides structured controls for identity and discovery fields, Markdown description,
associated listings, media, feature sections, requirements, languages, accessibility claims, and
external links. It does not expose raw event JSON. Complex repeatable fields use explicit typed row
formats in the UI and are converted to the v1 protocol model before core validation.

Draft validation calls the core Store Page builder and content policy. Core errors are returned as
typed diagnostics with stable codes and publisher-facing messages. The UI may report malformed
editor-row syntax, but it does not duplicate protocol limits, URL admission, Markdown sanitization,
media-role rules, association validation, or event-content sizing.

### 15.6 Cloning

Clone is a local draft operation until explicit publication. It:

- copies all v1 presentation content and compact discovery fields;
- assigns the publisher-selected new presentation ID;
- clears all listing associations;
- clears the loaded Store Page event ID;
- requires explicit listing links before validation and publication.

The clone command never signs or publishes an event. Creation concurrency rules apply when the
cloned draft is eventually published, so an existing event at the new coordinate rejects the create.

### 15.7 Reciprocal multi-event publication

The final Store Page `a` coordinates must exactly equal the listing mutations marked `link`. Listings
marked `unlink` are omitted from the replacement Store Page. Publication proceeds as follows:

1. Resolve the active signer and require its key to equal the requested publisher.
2. Validate the draft and exact desired association set in core.
3. Fetch the newest valid Store Page replacement and every current signed listing.
4. Verify kind, signature, author, canonical coordinate, and loaded event IDs.
5. Recheck Store Page concurrency and the active signer immediately before signing.
6. Build, sign, parse, and publish the Store Page replacement.
7. Confirm the exact Store Page event ID on the relay threshold and cache the signed event.
8. Refetch each listing immediately before its pointer mutation.
9. Preserve listing content and every non-`store_page` tag, remove all old Store Page pointer tags,
   and add exactly one canonical pointer for a link or none for an unlink.
10. Sign, publish, and confirm each exact listing replacement independently.

At least one relay acceptance means that event publication may be irreversible. Propagation is
confirmed only when the exact new event ID, not merely the coordinate, is observed on the configured
two-relay threshold.

### 15.8 Optimistic concurrency

Editor state retains the loaded Store Page event ID and the loaded event ID of every associated
listing. A missing Store Page event ID means create-only: publication is rejected if a current valid
Store Page already exists at that coordinate. An existing-page edit requires an exact ID match.

All selected listings are checked before Store Page publication. Every listing is checked again just
before pointer signing, after Store Page publication may already have occurred. A changed event is
reported as a stale partial result and is never silently overwritten. Account identity is also
rechecked before Store Page signing and before each listing signature.

### 15.9 Partial success and retry

The publish response reports Store Page relay acceptance and propagation separately from every
listing pointer result. Complete success requires exact-event propagation confirmation for the Store
Page and every requested listing replacement. The response remains partial when:

- the Store Page was accepted but propagation was not confirmed;
- one or more listing replacements failed to sign or publish;
- only some listing pointers propagated;
- a listing changed after preflight;
- a listing replacement was accepted but its propagation was not confirmed.

Retry validates that the already-published Store Page event is still current and does not recreate or
resign it. It rechecks Store Page propagation, retries only failed pointer mutations, and only
rechecks propagation for an already-published listing replacement when its event ID is known. If any
coordinate has moved to another replacement, retry reports a conflict instead of overwriting it.

### 15.10 Preview and draft lifecycle

Preview sends the structured draft through the same core validation, URL policy, Markdown sanitizer,
media filtering, and platform-requirement projection used by published Store Pages. The bridge alone
constructs the safe HTML wrapper, and the preview renders `StorePageRichDetail`, the buyer-facing
detail component. Preview labels itself clearly, displays commerce copied only from the selected
authoritative listing, suppresses external navigation, and exposes no purchase, claim, install, or
ownership actions.

Drafts are local and never published implicitly. In-memory Publisher Studio draft state survives
navigation for the same publisher and listing. It is scoped to the active publisher, cleared when the
publisher account changes, and protected by account-plus-generation checks so late loads cannot
overwrite user edits. Validation failures preserve the current draft. Leaving a dirty editor requires
explicit discard confirmation. Persistent draft storage remains deferred.

## 16. Version 1 limits

The 128 KiB complete-event ceiling matches a common relay envelope; the lower content ceiling leaves
space for signatures, tags, and JSON event metadata. Limits are enforced before expensive parsing or
allocation where practical.

| Field | Limit |
|---|---:|
| Complete serialized event | 128 KiB |
| Raw event content | 96 KiB |
| Title | 120 characters |
| Summary | 300 characters |
| Generic text field | 4,096 characters |
| Identifier/tag value | 128 characters |
| Description Markdown source | 64 KiB |
| Description sanitized HTML | 96 KiB |
| Media items | 40 |
| Trailers | 4 |
| Screenshots | 24 |
| Feature sections | 12 |
| Section Markdown source | 4 KiB each |
| Section sanitized HTML | 8 KiB each |
| Associated listings | 16 |
| Genres | 8 |
| Features | 16 |
| Languages | 64 |
| Accessibility entries | 32 |
| External links | 12 |
| URL length | 2,048 bytes |
| Markdown nesting | 12 levels |
| Markdown heading | H3 |
| Markdown links | 64 |
| Markdown images | 16 |

Builders enforce all limits available before signing; parsers additionally enforce the complete
serialized signed-event limit.

## 17. Implementation plan

### Gate 0 — Protocol allocation

- Select and centralize a non-conflicting experimental kind.
- Add collision regression tests.
- Document provisional capability reporting.

### Gate 1 — Security contract

- Select Markdown parser and sanitizer.
- Define the exact allowlist and URL policy.
- Define supported video formats.
- Add desktop and browser XSS regression fixtures.
- Review CSP and Tauri global bridge exposure.

### Phase 1 — Protocol and core models

- Define schema types, compact tags, normalized parser, validator, and builder.
- Implement multi-listing association invariants.
- Implement explicit content/tag precedence.
- Reuse centralized replacement ordering.
- Add adversarial parser and replacement tests.

### Phase 2 — Discovery and cache

- Add pointer lookup, `#a` recovery discovery, and grouped batch lookup.
- Add Store Page and association cache migrations.
- Add repository APIs and IPC-safe models.
- Protect against stale requests and account changes.

### Phase 3 — Store and Browse enrichment

- Add normalized presentation resolver and enriched card model.
- Deduplicate shared pages across platform listings.
- Use capsule, title, summary, genres, features, and release date.
- Preserve all listing-derived commerce and compatibility state.

### Phase 4 — Game Detail rendering

- Introduce distinct presentation and commerce models.
- Add media carousel, safe trailers, sanitized Markdown, sections, requirements, languages, accessibility, and links.
- Restrict requirements to authoritative listing platforms.
- Preserve buy, claim, ownership, and install flows unchanged.

### Phase 5 — Publisher editor and preview

- Add Associated Listings and Store Page sections.
- Implement link, clone, create, and unlink workflows.
- Reuse buyer-facing components for preview.
- Add optimistic concurrency and account-author checks.
- Handle partial publication and pointer repair.

### Phase 6 — Hardening

- Measure event size and relay compatibility.
- Add malformed-media, unsupported-schema, stale-cache, and relay-failure tests.
- Audit keyboard controls, focus, alt text, and reduced-motion behavior.
- Document migration strategy and experimental-kind replacement.

## 18. Likely codebase areas

| Area | Responsibility |
|---|---|
| `core/src/adp_protocol.rs` or dedicated constants module | Experimental kind constant and collision assertions |
| `core/src/store_page.rs` | Schema, compact tags, parser, normalization, validation, builder |
| `core/src/store_page_discovery.rs` | Pointer lookup, `#a` recovery, grouped relay discovery |
| `core/src/store_page_repository.rs` | Signed-event and policy-versioned sanitized cache |
| `core/src/replaceable_event.rs` | Reused ordering only; no Store Page-specific resolver |
| `core/migrations/010_store_pages.sql` | Store Page event and sanitized-content cache |
| `core/migrations/011_store_page_listing_events.sql` | Signed listing cache for offline reciprocal validation |
| `app/src/models.rs` | IPC-safe Store Page, media, association, and presentation models |
| `app/src/tauri_bridge.rs` | Fetch, publish, link, clone, and repair wrappers |
| `desktop/src/store_page_commands.rs` | Signer-aware Store Page and pointer commands |
| `desktop/src/main.rs` | Command registration |
| `app/src/ui_v2/views/marketplace_loader.rs` | Visible-window enrichment and page deduplication |
| `app/src/ui_v2/components/game_card.rs` | Capsule, summary, and discovery metadata |
| `app/src/ui_v2/views/game_detail.rs` | Rich presentation with commerce isolation |
| `app/src/ui_v2/views/publish.rs` | Store Page navigation and association workflow |
| `app/src/ui_v2/components/store_page_editor/` | Structured editor, sanitization feedback, preview |

## 19. Test matrix

| Test group | Required cases |
|---|---|
| Kind safety | Collision with assigned/project kinds; constant used consistently |
| Identity | Wrong author; zero/multiple `d`; invalid `a`; foreign publisher; duplicate associations |
| Two-way association | Pointer only; `a` only; mismatched page; stale pointer; valid bidirectional link |
| Replacement | Newer timestamp; equal tie-break; older ignored; invalid newer does not replace valid cache |
| Schema | Unknown fields; unsupported version; oversized values; duplicate media IDs; bad section references |
| Precedence | Conflicting JSON/tag values resolve exactly as specified |
| Security | Script URLs; encoded schemes; raw HTML; event handlers; iframe payloads; malformed video URLs |
| Fallbacks | No page; malformed page; unsupported page; partial media; missing capsule; missing description |
| Multi-platform | Three listings share one page; one listing unlinked; platform requirements filtered correctly |
| Marketplace | Cached enrichment; fresh update; visible-window batches; shared-page deduplication; stale response rejection |
| Detail | Independent listing/page refresh failures; commerce actions unchanged |
| Publisher | Link existing; clone forward; changed listing `d`; partial pointer failure; remote concurrent edit |
| Regression | Purchases, claims, campaigns, ownership, install, platform filtering, and ADP behavior remain listing-derived |

## 20. Remaining product decisions

These decisions remain, but they do not alter the core architecture:

1. Exact experimental event kind after Gate 0 verification.
2. Whether `release_date` supports only dates or also structured states such as `coming-soon` and `early-access`.
3. Initial normalized genre and feature vocabulary while preserving unknown values.
4. Exact Markdown library and sanitization implementation after the security gate.
5. Exact direct video formats supported in version 1.
6. Practical event-size limit across Arcadestr's supported relays.
7. Whether unpublished drafts live only in UI state or in a dedicated local draft table.
8. Store Page refresh TTL and visible-window batch size.
9. Whether one listing may intentionally reference more than one Store Page in a future version; version 1 permits at most one active pointer.

## 21. Final recommendation

Proceed with a separate, game-scoped Store Page parameterized replaceable event capable of enriching multiple `kind:30402` listings from the same publisher.

Use an advisory listing pointer plus a matching Store Page `a` reference as a required two-way association. Keep the listing authoritative for every commercial, access, platform, build, ownership, and distribution decision. Apply Store Page data only after signature, identity, association, schema, sanitization, and replacement validation.

Implement cached-first visible-window enrichment for Store and Browse, independent presentation refresh for Game Detail, and a Publisher Studio workflow that supports linking an existing page, cloning forward, creating a new page, and repairing partial pointer updates.

The event-kind collision check and untrusted-content sanitization policy are pre-implementation gates, not deferred open questions.

## Appendix A — Compact data flow

```text
Publisher flow

Select or publish listing(s)
        ↓
Choose Store Page association
├── Link existing
├── Clone existing
└── Create new
        ↓
Edit local presentation draft
        ↓
Validate + sanitize + preview
        ↓
Publish Store Page replacement
        ↓
Update listing store_page pointer(s)
        ↓
Confirm propagation independently
        ↓
Cache page + validated associations

Marketplace flow

Listing cache
        ↓
Render ordinary cards
        ↓
Visible listing window
        ↓
Parse and deduplicate Store Page pointers
        ↓
Cached Store Pages
        ↓
Validate two-way association
        ↓
Apply presentation enrichment
        ↓
Batch relay refresh
        ↓
Resolve replacement + persist + update cards

Game Detail flow

Authoritative listing + optional Store Page
        ↓
GameCommerceState      GamePresentation
        ↓                      ↓
Buy/claim/install      Hero/media/description/requirements
        └──────────── merged UI only ────────────┘
```
