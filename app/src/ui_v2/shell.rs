use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::models::{npub_fallback_label, GameListing};
use crate::relay_state::{apply_relay_event, merge_relay_snapshot};
use crate::ui_v2::components::{
    create_game_dirty, guard_navigation, set_create_game_dirty, NavigationGuard, PageContainer,
    TopBar, UnsavedChangesDialog, UnsavedWork, ROUTE_FOCUS_FALLBACK_ID,
};
use crate::ui_v2::theme::UI_V2_STYLES;
use crate::ui_v2::views::store_page_publish::{
    discard_publisher_store_page_drafts, publisher_store_page_dirty_coordinates,
};
use crate::ui_v2::views::{
    AchievementsView, BrowseGamesView, BrowseRequest, GameDetailView, LibraryView, ProfileV2View,
    PublishV2View, PublishViewState, PurchasesView, SettingsView, SocialView, StoreFrontView,
};
use crate::{invoke_get_allow_insecure_public_ws, invoke_get_connected_relays, AuthContext};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DetailOrigin {
    Store,
    Library,
    Publisher,
}

fn detail_back_destination(origin: DetailOrigin) -> UiV2View {
    match origin {
        DetailOrigin::Store => UiV2View::Store,
        DetailOrigin::Library => UiV2View::Library,
        DetailOrigin::Publisher => UiV2View::Publish(PublishViewState::Games),
    }
}

/// Which editor, if any, the current route is showing with unsaved input.
///
/// Only the editor actually on screen can have unsaved input to lose, so a
/// dirty Store Page draft for some other listing never blocks navigation from
/// an unrelated screen.
fn unsaved_work_for_route(
    view: &UiV2View,
    create_game_is_dirty: bool,
    store_page_dirty: bool,
) -> Option<UnsavedWork> {
    match view {
        UiV2View::Publish(PublishViewState::NewPublication) if create_game_is_dirty => {
            Some(UnsavedWork::CreateGame)
        }
        UiV2View::Publish(PublishViewState::StorePage(_)) if store_page_dirty => {
            Some(UnsavedWork::StorePage)
        }
        _ => None,
    }
}

/// What the shell will do once the unsaved-navigation guard is answered.
#[derive(Clone, PartialEq)]
enum PendingNavigation {
    Route(UiV2View),
    SignOut,
}

/// A navigation to the same route is not a navigation and is never guarded.
fn is_route_change(from: &UiV2View, to: &UiV2View) -> bool {
    from != to
}

fn should_reset_publisher_route(account_changed: bool, view: &UiV2View) -> bool {
    account_changed
        && matches!(
            view,
            UiV2View::Publish(state) if !matches!(state, PublishViewState::Games)
        )
}

#[derive(Clone, PartialEq)]
enum UiV2View {
    Store,
    Browse(BrowseRequest),
    Detail(GameListing, DetailOrigin),
    Library,
    Social,
    Achievements,
    Purchases,
    Publish(PublishViewState),
    Profile,
    Settings,
}

#[cfg(test)]
mod tests {
    use super::{
        detail_back_destination, is_route_change, should_reset_publisher_route,
        unsaved_work_for_route, BrowseRequest, DetailOrigin, PublishViewState, UiV2View,
    };
    use crate::ui_v2::components::{guard_navigation, NavigationGuard, UnsavedWork};

    /// The `UiV2Root` body only. In this file the test module precedes the
    /// component, so assertions must not read their own literals back.
    fn root_source() -> &'static str {
        // `concat!` so this marker is not itself a literal in the file being
        // scanned; the test module precedes the component.
        include_str!("shell.rs")
            .split(concat!("pub fn Ui", "V2Root"))
            .nth(1)
            .expect("UiV2Root component source")
    }

    #[test]
    fn leaving_a_dirty_create_game_form_is_guarded() {
        let view = UiV2View::Publish(PublishViewState::NewPublication);
        assert_eq!(
            unsaved_work_for_route(&view, true, false),
            Some(UnsavedWork::CreateGame)
        );
        assert_eq!(
            guard_navigation(unsaved_work_for_route(&view, true, false)),
            NavigationGuard::Confirm(UnsavedWork::CreateGame)
        );
    }

    #[test]
    fn leaving_a_dirty_store_page_editor_is_guarded() {
        let view = UiV2View::Publish(PublishViewState::StorePage(listing()));
        assert_eq!(
            unsaved_work_for_route(&view, false, true),
            Some(UnsavedWork::StorePage)
        );
    }

    #[test]
    fn clean_editors_never_block_navigation() {
        assert_eq!(
            unsaved_work_for_route(
                &UiV2View::Publish(PublishViewState::NewPublication),
                false,
                false
            ),
            None
        );
        assert_eq!(
            unsaved_work_for_route(
                &UiV2View::Publish(PublishViewState::StorePage(listing())),
                false,
                false
            ),
            None
        );
        assert_eq!(guard_navigation(None), NavigationGuard::Proceed);
    }

    #[test]
    fn dirty_state_in_one_editor_does_not_block_navigation_from_another_screen() {
        // A dirty Store Page draft for some other listing must not guard the
        // Library or the Store.
        assert_eq!(unsaved_work_for_route(&UiV2View::Library, true, true), None);
        assert_eq!(unsaved_work_for_route(&UiV2View::Store, true, true), None);
        assert_eq!(
            unsaved_work_for_route(&UiV2View::Publish(PublishViewState::Games), true, true),
            None
        );
    }

    #[test]
    fn navigating_to_the_current_route_is_not_a_navigation() {
        assert!(!is_route_change(&UiV2View::Store, &UiV2View::Store));
        assert!(is_route_change(&UiV2View::Store, &UiV2View::Library));
    }

    #[test]
    fn signing_out_of_a_dirty_editor_is_guarded_like_any_other_navigation() {
        // Sign-out routes through the same interception point, so the guard
        // decision is identical to a route change.
        let view = UiV2View::Publish(PublishViewState::NewPublication);
        assert_eq!(
            guard_navigation(unsaved_work_for_route(&view, true, false)),
            NavigationGuard::Confirm(UnsavedWork::CreateGame)
        );
        let source = root_source();
        assert!(source.contains("PendingNavigation::SignOut"));
        assert!(source.contains("request_navigation.run(PendingNavigation::SignOut)"));
    }

    #[test]
    fn every_shell_navigation_callback_routes_through_the_guard() {
        let source = root_source();
        // Only three places may set the route directly: the account-switch
        // publisher reset, and the two guard resolutions. Every user-facing
        // navigation callback must go through `navigate_guarded`.
        let direct_sets = source.matches("current_view.set(").count();
        assert_eq!(
            direct_sets, 3,
            "unguarded current_view.set found; route every navigation through navigate_guarded"
        );
        for callback in [
            "navigate_store",
            "navigate_browse",
            "navigate_library",
            "navigate_social",
            "navigate_publish",
            "navigate_profile",
            "navigate_achievements",
            "navigate_purchases",
            "navigate_settings",
        ] {
            let body = source
                .split(&format!("let {callback} ="))
                .nth(1)
                .unwrap_or_else(|| panic!("{callback} should exist"));
            let body = &body[..body.find(";\n").unwrap_or(body.len())];
            assert!(
                body.contains("navigate_guarded.run("),
                "{callback} must route through the navigation guard"
            );
        }
    }

    #[test]
    fn discard_and_continue_reaches_the_pending_route() {
        let source = root_source();
        assert!(source.contains("on_discard_and_continue"));
        assert!(source.contains("discard_publisher_store_page_drafts"));
        assert!(source.contains("set_create_game_dirty(false)"));
    }

    #[test]
    fn keep_editing_drops_the_pending_route_and_stays_put() {
        let source = root_source();
        let handler = source
            .split("let on_keep_editing = Callback::new(move |_| {")
            .nth(1)
            .expect("keep-editing handler should exist");
        let handler = &handler[..handler.find("\n    });").expect("handler should close")];
        assert!(handler.contains("pending_work.set(None)"));
        assert!(handler.contains("pending_navigation.set(None)"));
        // Keeping the user in the editor must never move the route.
        assert!(!handler.contains("current_view"));
    }

    fn listing() -> crate::models::GameListing {
        use crate::models::{AcquisitionPolicy, GameListing, ListingSource};

        GameListing {
            id: "listing".to_string(),
            source: ListingSource::Nip99Listing,
            title: "Game".to_string(),
            description: String::new(),
            images: Vec::new(),
            download_url: String::new(),
            price: 0.0,
            currency: "SATS".to_string(),
            price_sats: 0,
            quantity: None,
            tags: Vec::new(),
            specs: Vec::new(),
            publisher_npub: "npub1publisher".to_string(),
            stall_id: String::new(),
            stall_name: None,
            lud16: String::new(),
            event_id: None,
            created_at: 0,
            platforms: Vec::new(),
            nip94_event_id: None,
            acquisition: AcquisitionPolicy::default(),
            campaigns: Vec::new(),
            is_owned: false,
            #[cfg(debug_assertions)]
            nip99_raw_event_json: None,
        }
    }

    #[test]
    fn shell_does_not_use_generated_identity_or_fake_controls() {
        let source = include_str!("shell.rs");

        assert!(!source.contains(concat!("aida", "-public")));
        assert!(!source.contains(concat!("Za", "ps")));
        assert!(!source.contains(concat!("Notifica", "tions")));
        assert!(!source.contains(concat!("Sup", "port")));
    }

    #[test]
    fn every_shell_destination_remains_reachable() {
        let source = include_str!("shell.rs");
        for callback in [
            "navigate_store",
            "navigate_browse",
            "navigate_library",
            "navigate_social",
            "navigate_publish",
            "navigate_profile",
            "navigate_achievements",
            "navigate_purchases",
            "navigate_settings",
        ] {
            assert!(
                source.contains(callback),
                "missing shell navigation callback {callback}"
            );
        }
        assert!(!source.contains(concat!("Mobile", "NavItem")));
        assert!(!source.contains(concat!("mobile-primary", "-navigation")));
    }

    #[test]
    fn store_category_payload_reaches_browse_navigation_state() {
        let view = UiV2View::Browse(BrowseRequest::for_category("Action RPG"));
        let UiV2View::Browse(request) = view else {
            panic!("category navigation must target Browse");
        };
        assert_eq!(request.category.as_deref(), Some("action rpg"));
    }

    #[test]
    fn detail_back_navigation_preserves_its_origin() {
        assert!(matches!(
            detail_back_destination(DetailOrigin::Store),
            UiV2View::Store
        ));
        assert!(matches!(
            detail_back_destination(DetailOrigin::Library),
            UiV2View::Library
        ));
        assert!(matches!(
            detail_back_destination(DetailOrigin::Publisher),
            UiV2View::Publish(PublishViewState::Games)
        ));
    }

    #[test]
    fn account_switch_resets_contextual_publisher_routes() {
        assert!(should_reset_publisher_route(
            true,
            &UiV2View::Publish(PublishViewState::NewPublication)
        ));
        assert!(!should_reset_publisher_route(
            true,
            &UiV2View::Publish(PublishViewState::Games)
        ));
        assert!(!should_reset_publisher_route(false, &UiV2View::Store));
    }

    #[test]
    fn profile_achievements_and_settings_navigation_remain_available() {
        let source = include_str!("shell.rs");
        for destination in [
            "ProfileV2View",
            "AchievementsView",
            "PurchasesView",
            "SettingsView",
        ] {
            assert!(source.contains(destination));
        }
    }

    #[test]
    fn old_inline_settings_markup_is_removed() {
        let source = include_str!("shell.rs");
        assert!(!source.contains(concat!("Allow insecure public ", "ws:// relays")));
        assert!(source.contains("<SettingsView"));
    }
}

#[component]
pub fn UiV2Root(relay_count: RwSignal<usize>) -> impl IntoView {
    let auth = use_context::<AuthContext>().expect("AuthContext not provided");
    let current_view = RwSignal::new(UiV2View::Store);
    let connected_relays = RwSignal::new(Vec::<String>::new());
    let relay_snapshot_loaded = RwSignal::new(false);
    let pending_relay_events = RwSignal::new(Vec::<(String, String)>::new());
    let allow_insecure_public_ws = RwSignal::new(false);
    let settings_error = RwSignal::new(None::<String>);
    let publisher_route_account = RwSignal::new(auth.npub.get_untracked());

    let auth_for_publisher_route = auth.clone();
    Effect::new(move |_| {
        let current_account = auth_for_publisher_route.npub.get();
        let account_changed = publisher_route_account.get_untracked() != current_account;
        if account_changed {
            publisher_route_account.set(current_account);
        }
        if should_reset_publisher_route(account_changed, &current_view.get_untracked()) {
            current_view.set(UiV2View::Publish(PublishViewState::Games));
        }
    });

    Effect::new(move |_| {
        let connected_relays_for_effect = connected_relays;
        let relay_count_for_effect = relay_count;
        spawn_local(async move {
            #[cfg(not(feature = "web"))]
            {
                let relays_for_listener = connected_relays_for_effect;
                let relay_count_for_listener = relay_count_for_effect;

                match crate::tauri_invoke::listen("relay-connection", move |payload| {
                    let event_type = payload
                        .get("type")
                        .and_then(|value| value.as_str())
                        .unwrap_or_default();
                    let url = payload
                        .get("url")
                        .and_then(|value| value.as_str())
                        .unwrap_or_default();

                    relays_for_listener.update(|relays| {
                        let _ = apply_relay_event(relays, event_type, url);
                    });
                    if !relay_snapshot_loaded.get_untracked() {
                        pending_relay_events.update(|events| {
                            events.push((event_type.to_string(), url.to_string()));
                        });
                    }
                    relay_count_for_listener.set(relays_for_listener.get_untracked().len());
                })
                .await
                {
                    Ok(cleanup) => {
                        std::mem::forget(cleanup);
                    }
                    Err(err) => {
                        web_sys::console::error_1(
                            &format!("[UiV2Root] Failed to subscribe relay listener: {err}").into(),
                        );
                    }
                }
            }

            match invoke_get_connected_relays().await {
                Ok(snapshot_relays) => {
                    let mut reconciled = Vec::new();
                    merge_relay_snapshot(&mut reconciled, snapshot_relays);
                    for (event_type, url) in pending_relay_events.get_untracked() {
                        let _ = apply_relay_event(&mut reconciled, &event_type, &url);
                    }
                    connected_relays_for_effect.set(reconciled);
                    relay_snapshot_loaded.set(true);
                    pending_relay_events.set(Vec::new());
                    relay_count_for_effect.set(connected_relays_for_effect.get_untracked().len());
                }
                Err(err) => {
                    web_sys::console::error_1(
                        &format!("[UiV2Root] Failed to load relay snapshot: {err}").into(),
                    );
                }
            }
        });
    });

    Effect::new(move |_| {
        spawn_local(async move {
            match invoke_get_allow_insecure_public_ws().await {
                Ok(value) => allow_insecure_public_ws.set(value),
                Err(err) => settings_error.set(Some(err)),
            }
        });
    });

    let relay_count_signal = Signal::derive(move || relay_count.get());
    let connected_relays_signal = Signal::derive(move || connected_relays.get());
    let connection_status = Signal::derive(move || auth.connection_status.get());
    let connection_error = Signal::derive(move || auth.connection_error.get());
    let display_name = Signal::derive(move || {
        auth.profile
            .get()
            .map(|profile| profile.display())
            .or_else(|| {
                auth.active_account
                    .get()
                    .and_then(|account| account.display_name.or(account.name).or(account.username))
            })
            .or_else(|| auth.npub.get().map(|npub| npub_fallback_label(&npub)))
            .unwrap_or_else(|| "No active account".to_string())
    });
    let avatar_url = Signal::derive(move || {
        auth.profile
            .get()
            .and_then(|profile| profile.picture)
            .or_else(|| {
                auth.active_account
                    .get()
                    .and_then(|account| account.picture)
            })
    });
    let avatar_fallback = Signal::derive(move || {
        display_name
            .get()
            .chars()
            .find(|character| character.is_alphanumeric())
            .map(|character| character.to_uppercase().to_string())
            .unwrap_or_else(|| "?".to_string())
    });

    // ── Unsaved-navigation guard ────────────────────────────────────────
    // Every shell route change funnels through `navigate_guarded`, so primary
    // navigation, publisher tab navigation, route replacement, detail back,
    // and sign-out are all covered by one interception point.
    let pending_navigation = RwSignal::new(None::<PendingNavigation>);
    let pending_work = RwSignal::new(None::<UnsavedWork>);

    let auth_for_guard = StoredValue::new_local(auth.clone());
    let unsaved_for_current_route = move || {
        let publisher = auth_for_guard
            .get_value()
            .npub
            .get_untracked()
            .unwrap_or_default();
        let store_page_dirty =
            !publisher.is_empty() && !publisher_store_page_dirty_coordinates(&publisher).is_empty();
        unsaved_work_for_route(
            &current_view.get_untracked(),
            create_game_dirty(),
            store_page_dirty,
        )
    };

    let auth_for_signout = StoredValue::new_local(auth.clone());
    let perform_sign_out = move || {
        let auth_ctx = auth_for_signout.get_value();
        spawn_local(async move {
            match auth_ctx.logout_nip46().await {
                Ok(_) => auth_ctx.error.set(None),
                Err(err) => auth_ctx.error.set(Some(err)),
            }
        });
    };

    let request_navigation = Callback::new(move |pending: PendingNavigation| {
        if let PendingNavigation::Route(target) = &pending {
            if !is_route_change(&current_view.get_untracked(), target) {
                return;
            }
        }
        match guard_navigation(unsaved_for_current_route()) {
            NavigationGuard::Proceed => match pending {
                PendingNavigation::Route(target) => current_view.set(target),
                PendingNavigation::SignOut => perform_sign_out(),
            },
            NavigationGuard::Confirm(work) => {
                pending_work.set(Some(work));
                pending_navigation.set(Some(pending));
            }
        }
    });

    let navigate_guarded = Callback::new(move |target: UiV2View| {
        request_navigation.run(PendingNavigation::Route(target));
    });

    let on_keep_editing = Callback::new(move |_| {
        pending_work.set(None);
        pending_navigation.set(None);
    });

    let on_discard_and_continue = Callback::new(move |_| {
        // Drop the unsaved input for the editor being left. This is the same
        // operation the editor's own Discard control performs.
        match pending_work.get_untracked() {
            Some(UnsavedWork::CreateGame) => set_create_game_dirty(false),
            Some(UnsavedWork::StorePage) => {
                if let Some(publisher) = auth_for_guard.get_value().npub.get_untracked() {
                    discard_publisher_store_page_drafts(&publisher);
                }
            }
            None => {}
        }
        let pending = pending_navigation.get_untracked();
        pending_work.set(None);
        pending_navigation.set(None);
        match pending {
            Some(PendingNavigation::Route(target)) => current_view.set(target),
            Some(PendingNavigation::SignOut) => perform_sign_out(),
            None => {}
        }
    });

    let navigate_store = Callback::new(move |_| navigate_guarded.run(UiV2View::Store));
    let navigate_browse = Callback::new(move |_| {
        navigate_guarded.run(UiV2View::Browse(BrowseRequest::default()));
    });
    let search_browse = Callback::new(move |query: String| {
        navigate_guarded.run(UiV2View::Browse(BrowseRequest::for_query(query)));
    });
    let navigate_store_category = Callback::new(move |request: BrowseRequest| {
        navigate_guarded.run(UiV2View::Browse(request));
    });
    let navigate_library = Callback::new(move |_| navigate_guarded.run(UiV2View::Library));
    let navigate_social = Callback::new(move |_| navigate_guarded.run(UiV2View::Social));
    let navigate_publish = Callback::new(move |_| {
        navigate_guarded.run(UiV2View::Publish(PublishViewState::Games));
    });
    let navigate_profile = Callback::new(move |_| navigate_guarded.run(UiV2View::Profile));
    let navigate_achievements =
        Callback::new(move |_| navigate_guarded.run(UiV2View::Achievements));
    let navigate_purchases = Callback::new(move |_| navigate_guarded.run(UiV2View::Purchases));
    let navigate_settings = Callback::new(move |_| navigate_guarded.run(UiV2View::Settings));

    let on_select_listing = Callback::new(move |listing: GameListing| {
        navigate_guarded.run(UiV2View::Detail(listing, DetailOrigin::Store));
    });
    let on_select_library_listing = Callback::new(move |listing: GameListing| {
        navigate_guarded.run(UiV2View::Detail(listing, DetailOrigin::Library));
    });
    let on_back_from_detail = Callback::new(move |_| {
        let origin = match current_view.get_untracked() {
            UiV2View::Detail(_, origin) => origin,
            _ => DetailOrigin::Store,
        };
        navigate_guarded.run(detail_back_destination(origin));
    });
    let on_open_publish_from_profile = Callback::new(move |_| {
        navigate_guarded.run(UiV2View::Publish(PublishViewState::Games));
    });
    let on_open_listing_from_profile = Callback::new(move |listing: GameListing| {
        navigate_guarded.run(UiV2View::Detail(listing, DetailOrigin::Store));
    });
    let on_disconnect = Callback::new(move |_| {
        request_navigation.run(PendingNavigation::SignOut);
    });

    let discover_active = Signal::derive(move || {
        matches!(
            current_view.get(),
            UiV2View::Store | UiV2View::Browse(_) | UiV2View::Detail(_, DetailOrigin::Store)
        )
    });
    let library_active = Signal::derive(move || {
        matches!(
            current_view.get(),
            UiV2View::Library | UiV2View::Detail(_, DetailOrigin::Library)
        )
    });
    let social_active = Signal::derive(move || current_view.get() == UiV2View::Social);
    let publish_active = Signal::derive(move || matches!(current_view.get(), UiV2View::Publish(_)));

    view! {
        <div class="arc-app-shell">
            <style>{UI_V2_STYLES}</style>

            <TopBar
                relay_count=relay_count_signal
                connected_relays=connected_relays_signal
                display_name=display_name
                avatar_url=avatar_url
                avatar_fallback=avatar_fallback
                connection_status=connection_status
                connection_error=connection_error
                discover_active=discover_active
                library_active=library_active
                community_active=social_active
                publish_active=publish_active
                on_open_store=navigate_store
                on_open_browse=navigate_browse
                on_search=search_browse
                on_open_library=navigate_library
                on_open_community=navigate_social
                on_open_publish=navigate_publish
                on_open_profile=navigate_profile
                on_open_achievements=navigate_achievements
                on_open_purchases=navigate_purchases
                on_open_settings=navigate_settings
                on_disconnect=on_disconnect
            />

            <main id=ROUTE_FOCUS_FALLBACK_ID tabindex="-1">
                <PageContainer wide=true full_height=true>
                    {move || match current_view.get() {
                        UiV2View::Store => view! {
                            <StoreFrontView on_select=on_select_listing on_browse=navigate_store_category />
                        }
                        .into_any(),
                        UiV2View::Browse(request) => view! {
                            <BrowseGamesView on_select=on_select_listing request=request />
                        }
                        .into_any(),
                        UiV2View::Detail(listing, _) => view! {
                            <GameDetailView listing=listing on_back=on_back_from_detail />
                        }
                        .into_any(),
                        UiV2View::Library => view! {
                            <LibraryView on_open_listing=on_select_library_listing />
                        }
                        .into_any(),
                        UiV2View::Social => view! { <SocialView /> }.into_any(),
                        UiV2View::Achievements => view! { <AchievementsView /> }.into_any(),
                        UiV2View::Purchases => view! { <PurchasesView /> }.into_any(),
                        UiV2View::Publish(state) => {
                            let on_navigate = Callback::new(move |state| {
                                navigate_guarded.run(UiV2View::Publish(state));
                            });
                            let on_open_listing = Callback::new(move |listing| {
                                navigate_guarded
                                    .run(UiV2View::Detail(listing, DetailOrigin::Publisher));
                            });
                            view! { <PublishV2View state=state on_navigate=on_navigate on_open_listing=on_open_listing /> }.into_any()
                        }
                        UiV2View::Profile => view! {
                            <ProfileV2View
                                on_open_publish=on_open_publish_from_profile
                                on_open_listing=on_open_listing_from_profile
                            />
                        }
                        .into_any(),
                        UiV2View::Settings => view! {
                            <SettingsView
                                connected_relays=connected_relays_signal
                                allow_insecure_public_ws=allow_insecure_public_ws
                                settings_error=settings_error
                                on_sign_out=on_disconnect
                            />
                        }
                        .into_any(),
                    }}
                </PageContainer>
            </main>

            <UnsavedChangesDialog
                work=Signal::derive(move || pending_work.get())
                on_keep_editing=on_keep_editing
                on_discard=on_discard_and_continue
            />
        </div>
    }
}
