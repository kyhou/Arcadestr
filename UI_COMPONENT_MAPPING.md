# UI Component Mapping

| Lovable page or component | Current Arcadestr equivalent | Status | Required backend data | Existing Tauri command or state source | Missing backend capability | Migration risks |
|---|---|---|---|---|---|---|
| `AppShell` | `app/src/ui_v2/shell.rs` | Restructure | Active profile, navigation state, relay status | `AuthContext`; relay snapshot/events | URL history if desired later | Preserve Profile and Achievements despite absent reference routes; reference mobile navigation is incomplete |
| Top bar | `app/src/ui_v2/components/topbar.rs` | Restyle | Profile, relay state | `ProfileStore`; relay state | Search and notifications | Never show unconditional green Online state |
| `PageHeader` | Repeated view headings | Create | None | None | None | Keep API Leptos-native and avoid React slot patterns |
| `GameCard` | Local cards in Store and Browse | Create | Listing, access policy, ownership, compatibility | `GameListing`; marketplace store | Real ratings and zap totals | Do not use mock USD prices, ratings, zaps, or Play actions |
| Storefront | `store_front.rs` | Restructure | Streamed listings and active campaigns | `MarketplaceStore`; marketplace loader | Real social activity and support totals | Preserve cache-first loading and omit fabricated activity |
| Browse | `browse_games.rs` | Restructure | Listing title, description, tags, price, policy, platform | Marketplace loader; platform bridge | None for loaded-page filtering | Search/sort must not break pagination or platform filtering |
| Game detail | `game_detail.rs` | Restructure | Listing, seller, ownership, campaigns, install state | Ownership, payment, campaign, install, and profile bridges | Real ratings/current players/notes | Reference hides essential asynchronous and error states |
| Acquisition panel | Existing detail purchase/claim/install panel | Restyle | Access policy, durable credential, active campaign | LNURL, NWC, purchase, entitlement, ownership, install bridges | None for current supported paths | Keep all four acquisition categories distinct |
| Technical disclosure | Existing specs/debug areas | Restructure | Event ID, listing coordinate, author, platform, fulfillment | `GameListing`; listing event metadata | Verified provider/file status where unavailable | Do not assert verification without evidence |
| Library | `library.rs` | Restructure | Installed-game registry | `invoke_get_installed_games` | Complete owned library, updates, launch, storage totals, verification | Do not synthesize ownership or implement unrelated lifecycle work |
| Community | `social.rs` | Restyle or omit mocks | Notes, profiles, reactions, zaps | None currently | Feed queries, note publishing, reactions, reposts, zaps | Static reference content must not ship as real data |
| Publish wizard | `components/publish.rs`; `ui_v2/views/publish.rs` | Restructure | Listing metadata, access policy, build, distribution, campaigns | ADP publication and campaign bridges | Draft persistence and screenshot upload | Preserve validation, hashing, progress, edit, and campaign flows |
| Accounts | Account-selection mode in `login.rs` | Restyle | Saved accounts and active account | `AuthContext`; desktop and web auth | None for existing flows | Reference omits bunker, NIP-07, QR, nsec, restore, and deletion |
| Settings account section | `profile.rs`; root profile state | Restructure | Profile metadata and NIP-05 | `ProfileStore`; profile and NIP-05 commands | Profile metadata editing command not identified | Do not render editable controls that cannot save |
| Settings network section | Inline settings in `shell.rs`; relay menu | Restructure | Relay snapshot and insecure-relay setting | Relay snapshot/events; insecure-relay command | Add/remove/toggle relay APIs | Reference server list and latency are fabricated |
| Settings security | Login/profile NIP-49 components | Reuse and restyle | Signer type, connection state, encrypted key export | Auth commands; NIP-49 bridge | None for export itself | Existing profile modal currently uses placeholder export data |
| Settings backup | `BackupManager` | Reuse only after backend verification | Account/settings backup | Legacy wrappers | Registered `create_backup`/`restore_backup` commands are absent | Exposing these controls would produce runtime failures |
| Purchases route | No current view | Create only after backend | Validated NIP-102 receipts and Entitlement Grants | Purchase and entitlement repositories | Typed list command and frontend model | Must exclude public and timed access because neither creates a durable credential |
| Achievements | `achievements.rs` | Restyle | Badge definitions, awards, profile badge selections | Badge cache/relay commands | Standalone web support | No reference equivalent; behavior must not be removed |
| Profile and publisher listings | `profile.rs` | Restyle | Active profile, NIP-05, badges, publisher listings | Profile and marketplace stores | Profile editing | No direct reference page; keep reachable |
| Relay indicator/menu | Shell and top bar relay state | Restyle | Relay URL, status, error, count | Relay events plus snapshot | Standalone web relay state | Reference always displays Online regardless of state |
| Root errors/404 | Per-view state | Create selectively | Local errors | View-local async state | Global routing error boundary | URL routing is a separate architectural change |
| Generated `components/ui/*` | None | Omit | None | None | None | React, Radix, and shadcn runtime code is prohibited |
| Generated cover images | Real listing image URLs | Omit | NIP-99 listing images | `GameListing.images` | None | Never replace relay data with reference assets |
| Mock games/accounts/social/settings | Real Arcadestr state | Omit | Existing stores and commands | Existing integrations | Feature-specific gaps above | Mock data would regress production behavior |

## Acquisition Invariants

Every migrated card, filter, detail panel, library state, and durable-record view must preserve these categories:

1. **Paid purchase:** a successful purchase produces durable ownership through a NIP-102 receipt.
2. **Claim-and-keep campaign:** a successful campaign claim produces durable ownership through an Entitlement Grant.
3. **Public access:** installation is allowed only while the listing's current public policy permits it; no durable ownership is created.
4. **Timed access:** installation is allowed only during the configured interval; no durable ownership is created.

Public and timed access must not appear as purchases, permanent claims, or owned credentials. A zero price must not be interpreted as public access.
