# Game Detail Consolidation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace placeholders with real `GameListing` data, absorb buy flow and seller profile from old `DetailView` into v2 `GameDetailView`, remove old component call.

**Architecture:** Single-file rewrite of `game_detail.rs`. Add imports for buy flow (`ZapRequest`, `ZapInvoice`, `invoke_request_invoice`, `AuthContext`) and seller profile (`ProfileRow`, `try_use_profile_store`, `invoke_fetch_profile`). Add RwSignal state for invoice flow. Restructure content to 8+4 grid matching Stitch template. No new files needed.

**Tech Stack:** Leptos (Rust/WASM), Tauri IPC bridge, NIP-57 Lightning zaps

---

### Task 1: Add imports and state signals

**Files:**
- Modify: `app/src/ui_v2/views/game_detail.rs:1-8`

- [ ] **Step 1: Update imports**

Replace the current imports with the full set needed:

```rust
use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::components::{BadgeEarnedModal, ProfileRow};
use crate::models::{BadgeAward, BadgeDefinition, EarnedBadgeSummary, GameListing, UserProfile, ZapInvoice, ZapRequest};
use crate::store::try_use_profile_store;
use crate::{invoke_fetch_profile, invoke_request_invoice, AuthContext};
```

- [ ] **Step 2: Add state signals**

After `let close_badge_modal = { ... };`, add the buy flow and seller profile signals:

```rust
// ── Buy flow state ──
let invoice: RwSignal<Option<ZapInvoice>> = RwSignal::new(None);
let buy_loading: RwSignal<bool> = RwSignal::new(false);
let buy_error: RwSignal<Option<String>> = RwSignal::new(None);
let show_invoice: RwSignal<bool> = RwSignal::new(false);

// ── Seller profile state ──
let seller_profile: RwSignal<Option<UserProfile>> = RwSignal::new(None);
let profile_loading: RwSignal<bool> = RwSignal::new(true);
```

- [ ] **Step 3: Verify compilation**

Run: `cargo check -p arcadestr-app 2>&1 | head -40`
Expected: Compiles with new imports (may warn about unused signals — that's fine)

- [ ] **Step 4: Commit**

```bash
git add app/src/ui_v2/views/game_detail.rs
git commit -m "feat: add game detail imports and state signals"
```

---

### Task 2: Wire real data into hero section

**Files:**
- Modify: `app/src/ui_v2/views/game_detail.rs:34-82`

- [ ] **Step 1: Replace hero background image**

Old:
```rust
style="background-image: linear-gradient(to top, rgba(10,14,20,0.88), rgba(10,14,20,0.45)), url('https://lh3.googleusercontent.com/...'); background-size: cover; background-position: center;"
```

New — compute image URL at the top of the component (after `close_badge_modal`):

```rust
let hero_image = listing
    .images
    .first()
    .cloned()
    .unwrap_or_else(|| String::new());
```

Then in the `<header>` element:

```rust
style=format!("background-image: linear-gradient(to top, rgba(10,14,20,0.88), rgba(10,14,20,0.45)), url('{}'); background-size: cover; background-position: center;", hero_image)
```

- [ ] **Step 2: Replace kicker badge**

Old:
```rust
<p class="v2-store-kicker">"Masterpiece Edition"</p>
```

New:
```rust
<p class="v2-store-kicker">
    {listing.tags.first().cloned().unwrap_or_else(|| "Game".to_string())}
</p>
```

- [ ] **Step 3: Replace price in buy panel**

Old:
```rust
<div class="v2-detail-price">"84k Sats"</div>
<p class="v2-social-meta">"120k"</p>
```

New:
```rust
<div class="v2-detail-price">
    {if listing.price_sats > 0 {
        format!("{} Sats", listing.price_sats)
    } else {
        "Free".to_string()
    }}
</div>
```

Remove the strikethrough `<p class="v2-social-meta">"120k"</p>` line entirely.

- [ ] **Step 4: Replace developer and publisher**

Old:
```rust
<p class="v2-social-meta">"Developer: Luminescent Labs"</p>
<p class="v2-social-meta">"Publisher: Arcade Vault"</p>
```

New:
```rust
<p class="v2-social-meta">
    {format!("Developer: {}", listing.publisher_npub)}
</p>
<p class="v2-social-meta">
    {format!("Publisher: {}", listing.stall_name.as_deref().unwrap_or("Independent"))}
</p>
```

- [ ] **Step 5: Replace release date**

Old:
```rust
<p class="v2-social-meta">"Release Date: Oct 24, 2023"</p>
```

New — add a helper function near the top of the file (after imports):

```rust
fn format_timestamp(ts: u64) -> String {
    #[cfg(target_arch = "wasm32")]
    {
        let date = js_sys::Date::new(&(ts as f64 * 1000.0).into());
        let year = date.get_full_year();
        let month = date.get_month() + 1; // 0-indexed
        let day = date.get_date();
        format!("Release Date: {:02}/{:02}/{}", month, day, year)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        // Fallback for non-WASM: show year only
        let years_since_epoch = ts / 31_556_952; // avg seconds per year
        let year = 1970 + years_since_epoch;
        format!("Release Date: {}", year)
    }
}
```

Then replace the hardcoded line:
```rust
<p class="v2-social-meta">{format_timestamp(listing.created_at)}</p>
```

- [ ] **Step 6: Replace protocol/source badge**

Old:
```rust
<p class="v2-social-meta">"Protocol: NIP-01 / NIP-57"</p>
```

New:
```rust
<p class="v2-social-meta">
    {format!("Protocol: {}", match listing.source {
        crate::models::ListingSource::Nip15Product => "NIP-15",
        crate::models::ListingSource::Nip99Listing => "NIP-99",
        crate::models::ListingSource::Legacy => "NIP-01",
    })}
</p>
```

- [ ] **Step 7: Verify compilation**

Run: `cargo check -p arcadestr-app 2>&1 | head -50`
Expected: Compiles successfully

- [ ] **Step 8: Commit**

```bash
git add app/src/ui_v2/views/game_detail.rs
git commit -m "feat: wire real listing data into game detail hero"
```

---

### Task 3: Migrate buy flow into hero buy panel

**Files:**
- Modify: `app/src/ui_v2/views/game_detail.rs` (hero `<aside>` and its buttons)

- [ ] **Step 1: Add AuthContext and invoice handlers**

After the seller profile state signals, add:

```rust
let auth = use_context::<AuthContext>().expect("AuthContext not provided");

// Buy button handler
let on_buy = {
    let listing = listing.clone();
    let on_invoice_created = on_invoice_created.clone();

    Callback::new(move |()| {
        let buyer_npub = match auth.npub.get() {
            Some(n) => n,
            None => {
                buy_error.set(Some("Not authenticated".to_string()));
                return;
            }
        };

        let event_id = listing.event_id.clone().unwrap_or_else(|| listing.id.clone());

        let zap_req = ZapRequest {
            seller_npub: listing.publisher_npub.clone(),
            seller_lud16: listing.lud16.clone(),
            listing_event_id: event_id,
            amount_sats: listing.price_sats,
            buyer_npub,
            relays: vec![
                "wss://relay.damus.io".to_string(),
                "wss://relay.nostr.band".to_string(),
            ],
        };

        buy_loading.set(true);
        buy_error.set(None);
        show_invoice.set(false);

        let zap_req_for_async = zap_req.clone();
        spawn_local(async move {
            match invoke_request_invoice(zap_req_for_async).await {
                Ok(zap_invoice) => {
                    invoice.set(Some(zap_invoice));
                    show_invoice.set(true);
                    buy_loading.set(false);
                    on_invoice_created.run(());
                }
                Err(e) => {
                    buy_error.set(Some(e));
                    buy_loading.set(false);
                }
            }
        });
    })
};

// Copy invoice to clipboard
let on_copy_invoice = Callback::new(move |()| {
    #[cfg(target_arch = "wasm32")]
    {
        if let Some(inv) = invoice.get() {
            if let Some(window) = leptos::web_sys::window() {
                let _ = window.navigator().clipboard().write_text(&inv.bolt11);
            }
        }
    }
});

// Open in wallet
let on_open_wallet = Callback::new(move |()| {
    #[cfg(target_arch = "wasm32")]
    {
        if let Some(inv) = invoice.get() {
            let lightning_uri = format!("lightning:{}", inv.bolt11);
            if let Some(win) = leptos::web_sys::window() {
                let _ = win.location().set_href(&lightning_uri);
            }
        }
    }
});
```

- [ ] **Step 2: Replace the buy panel buttons with reactive invoice flow**

In the `<aside class="v2-detail-buy-panel">`, replace the static buttons:

Old:
```rust
<button class="v2-btn-secondary" on:click=move |_| on_back.run(())>
    "Back"
</button>
<button class="v2-btn-primary">
    "Buy with Lightning"
</button>
<button class="v2-btn-ghost">"Add to Library"</button>
```

New:
```rust
<button class="v2-btn-secondary" on:click=move |_| on_back.run(())>
    "Back"
</button>

{move || {
    if buy_loading.get() {
        view! {
            <button class="v2-btn-primary" disabled=true>
                "Requesting invoice..."
            </button>
        }.into_any()
    } else if show_invoice.get() {
        view! {
            <>
                <div class="v2-panel" style:padding="8px" style:font-size="0.8rem">
                    <p style:word-break="break-all">
                        {move || invoice.get().map(|inv| {
                            if inv.bolt11.len() > 40 {
                                format!("{}...", &inv.bolt11[..40])
                            } else {
                                inv.bolt11.clone()
                            }
                        }).unwrap_or_default()}
                    </p>
                </div>
                <button class="v2-btn-primary" on:click=move |_| on_copy_invoice.run(())>
                    "Copy Invoice"
                </button>
                <button class="v2-btn-secondary" on:click=move |_| on_open_wallet.run(())>
                    "Open in Wallet"
                </button>
            </>
        }.into_any()
    } else {
        view! {
            <>
                <button
                    class="v2-btn-primary"
                    on:click=move |_| on_buy.run(())
                    disabled=move || listing.price_sats == 0
                >
                    {if listing.price_sats > 0 {
                        format!("Buy with Lightning — {} sats", listing.price_sats)
                    } else {
                        "Free — Download".to_string()
                    }}
                </button>
                <button class="v2-btn-ghost">"Add to Library"</button>
            </>
        }.into_any()
    }
}}

{move || {
    buy_error.get().map(|err| {
        view! {
            <p class="v2-social-meta" style:color="var(--v2-danger)">{err}</p>
        }
    })
}}
```

- [ ] **Step 3: Verify compilation**

Run: `cargo check -p arcadestr-app 2>&1 | head -60`
Expected: Compiles successfully

- [ ] **Step 4: Commit**

```bash
git add app/src/ui_v2/views/game_detail.rs
git commit -m "feat: migrate buy flow into v2 game detail hero panel"
```

---

### Task 4: Add seller profile fetch and display

**Files:**
- Modify: `app/src/ui_v2/views/game_detail.rs` (add profile fetch effect, add seller section to right column)

- [ ] **Step 1: Add profile fetch effect**

After the buy flow callbacks, add:

```rust
// ── Fetch seller profile on mount ──
let publisher_npub_for_fetch = listing.publisher_npub.clone();
let profile_store_for_fetch = try_use_profile_store();

Effect::new(move |_| {
    let npub = publisher_npub_for_fetch.clone();
    let store = profile_store_for_fetch.clone();
    spawn_local(async move {
        profile_loading.set(true);

        let cached = store.as_ref().and_then(|s| s.get(&npub));
        if let Some(profile) = cached {
            seller_profile.set(Some(profile));
            profile_loading.set(false);
        } else {
            match invoke_fetch_profile(npub.clone(), None).await {
                Ok(profile) => {
                    if let Some(s) = &store {
                        s.put(profile.clone());
                    }
                    seller_profile.set(Some(profile));
                }
                Err(_) => seller_profile.set(None),
            }
            profile_loading.set(false);
        }
    });
});
```

- [ ] **Step 2: Verify compilation**

Run: `cargo check -p arcadestr-app 2>&1 | head -60`
Expected: Compiles successfully (profile signals used in next task)

- [ ] **Step 3: Commit**

```bash
git add app/src/ui_v2/views/game_detail.rs
git commit -m "feat: add seller profile fetch with caching"
```

---

### Task 5: Restructure content into 8+4 grid and wire gallery/specs

**Files:**
- Modify: `app/src/ui_v2/views/game_detail.rs` (replace the flat sections below hero)

- [ ] **Step 1: Replace the description block, grid, and transaction wrap**

Replace everything from `<section class="v2-panel v2-detail-description-block">` through the `</section>` for `v2-detail-transaction-wrap`, with the new 8+4 grid structure.

New structure:

```rust
<!-- 8+4 Content Grid -->
<div class="v2-detail-grid">
    <!-- Left: 8 cols equivalent — Gallery / Description / Feed -->
    <div class="v2-detail-feed">
        <!-- Gallery (images except first) -->
        {move || {
            let gallery_images: Vec<String> = listing.images.iter()
                .skip(1)
                .cloned()
                .collect();
            if !gallery_images.is_empty() {
                Some(view! {
                    <div class="v2-detail-gallery-grid">
                        {gallery_images.iter().take(3).map(|url| {
                            view! {
                                <img src={url.clone()} alt="screenshot" />
                            }
                        }).collect::<Vec<_>>()}
                        {if gallery_images.len() > 3 {
                            view! {
                                <div style:position="relative">
                                    <img src={gallery_images.get(3).cloned().unwrap_or_default()} alt="more media" />
                                    <div class="absolute inset-0 flex items-center justify-center bg-black/40" style:position="absolute; inset: 0; display: flex; align-items: center; justify-content: center; background: rgba(0,0,0,0.4);">
                                        <span class="text-on-surface font-bold" style:color="white; font-weight: 700;">
                                            {format!("+{} Media", gallery_images.len() - 3)}
                                        </span>
                                    </div>
                                </div>
                            }.into_any()
                        } else {
                            view! { <></> }.into_any()
                        }}
                    </div>
                }).into_any()
            } else {
                None
            }
        }}

        <!-- Description -->
        <div class="v2-section-header">
            <h3>{listing.title.clone()}</h3>
        </div>
        <p class="v2-hero-description">{listing.description.clone()}</p>
        <div class="v2-detail-tags">
            {listing.tags.iter().map(|tag| {
                view! { <span class="v2-chip">{tag.clone()}</span> }
            }).collect::<Vec<_>>()}
        </div>

        <!-- Nostr Feed (placeholder) -->
        <div class="v2-section-header" style:margin-top="2rem">
            <h3>"Nostr Feed"</h3>
            <button class="v2-btn-ghost">"Write a Note"</button>
        </div>
        <div class="v2-live-note v2-detail-note-card">
            <p class="v2-social-meta">"npub1...k9q2 - 2h ago"</p>
            <p>"Nostr-powered community reviews will appear here once relay subscriptions are implemented."</p>
            <div class="v2-social-actions">
                <span>"bolt —"</span>
                <span>"chat —"</span>
                <span>"sync —"</span>
            </div>
        </div>
    </div>

    <!-- Right: 4 cols equivalent — Specs + Seller Profile -->
    <div>
        <!-- Specs -->
        <section class="v2-panel v2-detail-specs" style:margin-bottom="1rem">
            <h3>"Specs"</h3>
            <div class="v2-spec-grid">
                {if listing.specs.is_empty() {
                    view! {
                        <>
                            <span>"OS"</span><span>"Cross-platform"</span>
                            <span>"Type"</span><span>"Digital Download"</span>
                        </>
                    }.into_any()
                } else {
                    listing.specs.iter().flat_map(|(key, value)| {
                        vec![
                            view! { <span>{key.clone()}</span> }.into_any(),
                            view! { <span>{value.clone()}</span> }.into_any(),
                        ]
                    }).collect::<Vec<_>>()
                }}
            </div>
        </section>

        <!-- Seller Profile -->
        <section class="v2-panel v2-detail-specs">
            <h3>"Developer"</h3>
            {move || {
                if profile_loading.get() {
                    view! { <p class="v2-social-meta">"Loading seller info..."</p> }.into_any()
                } else {
                    let npub = listing.publisher_npub.clone();
                    view! {
                        <div>
                            <ProfileRow
                                npub={npub.clone()}
                                avatar_size="48px"
                                truncate_npub=20
                            />
                            {move || seller_profile.get().map(|p| {
                                view! {
                                    <div style:margin-top="12px" style:padding-top="12px" style:border-top="1px solid var(--v2-outline-ghost)">
                                        {p.about.clone().map(|about| {
                                            if !about.is_empty() {
                                                let truncated = if about.len() > 120 {
                                                    format!("{}...", &about[..120])
                                                } else { about };
                                                view! {
                                                    <p class="v2-social-meta">{truncated}</p>
                                                }.into_any()
                                            } else { view! { <></> }.into_any() }
                                        })}
                                        {p.nip05.clone().map(|nip05| {
                                            view! {
                                                <p class="v2-social-meta">
                                                    {if p.nip05_verified { "✓ " } else { "? " }}{nip05}
                                                </p>
                                            }.into_any()
                                        })}
                                        {p.lud16.clone().map(|lud16| {
                                            if !lud16.is_empty() {
                                                view! {
                                                    <p class="v2-social-meta" style:color="var(--v2-primary)">
                                                        {"⚡ "}{lud16}
                                                    </p>
                                                }.into_any()
                                            } else { view! { <></> }.into_any() }
                                        })}
                                        {p.website.clone().map(|website| {
                                            view! {
                                                <a href={website.clone()} target="_blank" rel="noopener"
                                                   class="v2-social-meta"
                                                   style:color="var(--v2-secondary); text-decoration: none;">
                                                    {"🌐 "}{website}
                                                </a>
                                            }.into_any()
                                        })}
                                    </div>
                                }
                            })}
                        </div>
                    }.into_any()
                }
            }}
        </section>
    </div>
</div>
```

- [ ] **Step 2: Remove the old `<DetailView>` call**

Delete these lines from the bottom:
```rust
<section class="v2-panel v2-detail-transaction-wrap">
    <DetailView
        listing={listing}
        on_back={on_back}
        on_invoice_created=on_invoice_created
    />
</section>
```

- [ ] **Step 3: Verify compilation**

Run: `cargo check -p arcadestr-app 2>&1 | head -80`
Expected: Compiles successfully

- [ ] **Step 4: Commit**

```bash
git add app/src/ui_v2/views/game_detail.rs
git commit -m "feat: restructure content to 8+4 grid, wire gallery/specs, move seller profile"
```

---

### Task 6: Cleanup and final review

**Files:**
- Modify: `app/src/ui_v2/views/game_detail.rs`
- Verify: `app/src/ui_v2/theme.rs`

- [ ] **Step 1: Remove unused imports**

Check that `DetailView` is no longer imported and remove it from the imports line:
```rust
use crate::components::{BadgeEarnedModal, ProfileRow};
// (DetailView removed)
```

- [ ] **Step 2: Run full check**

Run: `cargo check -p arcadestr-app 2>&1`
Expected: No errors, no warnings

- [ ] **Step 3: Run clippy**

Run: `cargo clippy -p arcadestr-app 2>&1`
Expected: No warnings (or only pre-existing ones)

- [ ] **Step 4: Commit**

```bash
git add app/src/ui_v2/views/game_detail.rs
git commit -m "refactor: remove old DetailView call, clean up imports"
```

---

### Verification

- `cargo check -p arcadestr-app` — no errors
- `cargo clippy -p arcadestr-app` — no new warnings
- Manual review of the generated HTML should confirm:
  - Hero background uses `listing.images.first()`
  - Price shows real sats from `listing.price_sats`
  - Buy button triggers invoice flow
  - Seller profile renders with fetched Nostr data
  - Specs grid shows `listing.specs` key-value pairs
  - Gallery uses `listing.images`
  - Developer/Publisher/Release/Protocol show real data
  - No `<DetailView>` call remains
