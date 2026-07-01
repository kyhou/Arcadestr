# Marketplace Empty State Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Show clear empty-state banners on the Store front and Browse Games pages when no marketplace products are found.

**Architecture:** Keep the change in the existing Leptos view files. Store front can branch directly on `listings.get().is_empty()`. Browse Games should add a small pure helper for the empty-state visibility so the filter/loading conditions are testable.

**Tech Stack:** Rust, Leptos 0.8, Arcadestr app crate, existing Tailwind-style utility classes.

---

## File Structure

- Modify `app/src/ui_v2/views/store_front.rs`: render an inline banner instead of an empty trending grid when no listings are available.
- Modify `app/src/ui_v2/views/browse_games.rs`: add a helper for empty-state visibility, add tests for it, and render an inline banner instead of an empty browse grid.
- No new components or CSS files. The banner uses existing utility classes and design tokens.

## Task 1: Add Browse Games Empty-State Helper

**Files:**
- Modify: `app/src/ui_v2/views/browse_games.rs:434-441`
- Test: `app/src/ui_v2/views/browse_games.rs:804-820`

- [ ] **Step 1: Write the failing tests**

Add this test after `no_more_platform_message_only_shows_for_exhausted_filtered_view` in `app/src/ui_v2/views/browse_games.rs`:

```rust
    #[test]
    fn browse_empty_state_only_shows_after_loading_without_results() {
        assert!(show_browse_empty_state(0, false, false));
        assert!(!show_browse_empty_state(0, true, false));
        assert!(!show_browse_empty_state(0, false, true));
        assert!(!show_browse_empty_state(1, false, false));
    }
```

- [ ] **Step 2: Run the focused test to verify it fails**

Run:

```bash
rtk cargo test -p arcadestr-app browse_empty_state_only_shows_after_loading_without_results
```

Expected: FAIL because `show_browse_empty_state` does not exist.

- [ ] **Step 3: Add the helper**

Add this helper after `show_no_more_platform_message` in `app/src/ui_v2/views/browse_games.rs`:

```rust
fn show_browse_empty_state(displayed_count: usize, loading: bool, loading_more: bool) -> bool {
    displayed_count == 0 && !loading && !loading_more
}
```

- [ ] **Step 4: Run the focused test to verify it passes**

Run:

```bash
rtk cargo test -p arcadestr-app browse_empty_state_only_shows_after_loading_without_results
```

Expected: PASS.

- [ ] **Step 5: Commit the helper and test**

Run:

```bash
rtk git add app/src/ui_v2/views/browse_games.rs
rtk git commit -m "test: cover browse empty state visibility"
```

## Task 2: Render Store Front Empty-State Banner

**Files:**
- Modify: `app/src/ui_v2/views/store_front.rs:137-213`

- [ ] **Step 1: Add the empty-list branch**

In `StoreFrontView`, update the branch beginning at `} else {` after the error case so it checks for empty listings before rendering the grid:

```rust
                        } else if listings.get().is_empty() {
                            view! {
                                <div class="rounded-xl border border-outline-variant/15 bg-surface-container-high p-6 text-on-surface-variant">
                                    <p class="font-bold text-on-surface">"No products found"</p>
                                    <p class="mt-1 text-sm leading-relaxed">
                                        "We could not find any marketplace listings from the connected relays. Try again later or check your relay connection."
                                    </p>
                                </div>
                            }
                            .into_any()
                        } else {
```

Leave the existing grid rendering inside the final `else` unchanged.

- [ ] **Step 2: Check formatting**

Run:

```bash
rtk cargo fmt --check
```

Expected: PASS. If it fails, run `rtk cargo fmt` and repeat `rtk cargo fmt --check`.

- [ ] **Step 3: Commit the Store front banner**

Run:

```bash
rtk git add app/src/ui_v2/views/store_front.rs
rtk git commit -m "fix: show store empty state"
```

## Task 3: Render Browse Games Empty-State Banner

**Files:**
- Modify: `app/src/ui_v2/views/browse_games.rs:175-270`

- [ ] **Step 1: Add the empty-list branch**

In `BrowseGamesView`, update the branch beginning at `} else {` after the error case so it checks `displayed_listings` before rendering the grid:

```rust
                } else if show_browse_empty_state(
                    displayed_listings.get().len(),
                    loading.get(),
                    loading_more.get(),
                ) {
                    view! {
                        <div class="rounded-xl border border-outline-variant/15 bg-surface-container-high p-6 text-on-surface-variant">
                            <p class="font-bold text-on-surface">"No products found"</p>
                            <p class="mt-1 text-sm leading-relaxed">
                                "No games match the current marketplace results or filters."
                            </p>
                        </div>
                    }
                    .into_any()
                } else {
```

Leave the existing grid rendering inside the final `else` unchanged.

- [ ] **Step 2: Run the focused helper test**

Run:

```bash
rtk cargo test -p arcadestr-app browse_empty_state_only_shows_after_loading_without_results
```

Expected: PASS.

- [ ] **Step 3: Commit the Browse Games banner**

Run:

```bash
rtk git add app/src/ui_v2/views/browse_games.rs
rtk git commit -m "fix: show browse empty state"
```

## Task 4: Final Verification

**Files:**
- Verify: `app/src/ui_v2/views/store_front.rs`
- Verify: `app/src/ui_v2/views/browse_games.rs`

- [ ] **Step 1: Run formatting**

Run:

```bash
rtk cargo fmt --check
```

Expected: PASS.

- [ ] **Step 2: Run app crate check**

Run:

```bash
rtk cargo check -p arcadestr-app
```

Expected: PASS.

- [ ] **Step 3: Run focused tests**

Run:

```bash
rtk cargo test -p arcadestr-app browse_empty_state_only_shows_after_loading_without_results
```

Expected: PASS.

- [ ] **Step 4: Inspect git status**

Run:

```bash
rtk git status --short
```

Expected: no uncommitted changes from implementation tasks except intentional plan/spec files if they were not committed separately.

## Self-Review

- Spec coverage: Store front empty state is Task 2. Browse Games empty state and filtered-result behavior are Tasks 1 and 3. Verification is Task 4.
- Placeholder scan: No placeholders or deferred implementation steps remain.
- Type consistency: `show_browse_empty_state(displayed_count, loading, loading_more)` is defined before use and tested with the same signature.
