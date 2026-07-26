use leptos::prelude::*;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;

#[cfg(target_arch = "wasm32")]
const MOBILE_NAV_FOCUSABLE: &str =
    "button:not([disabled]), [href], [tabindex]:not([tabindex='-1'])";

use crate::models::{npub_fallback_label, GameListing};
use crate::relay_state::{apply_relay_event, merge_relay_snapshot};
use crate::ui_v2::components::{MobileNavItem, NavItem, TopBar};
use crate::ui_v2::theme::UI_V2_STYLES;
use crate::ui_v2::views::{
    AchievementsView, BrowseGamesView, BrowseRequest, GameDetailView, LibraryView, ProfileV2View,
    PublishV2View, PublishViewState, PurchasesView, SettingsView, SocialView, StoreFrontView,
};
use crate::{invoke_get_allow_insecure_public_ws, invoke_get_connected_relays, AuthContext};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DetailOrigin {
    Store,
    Library,
}

fn detail_back_destination(origin: DetailOrigin) -> UiV2View {
    match origin {
        DetailOrigin::Store => UiV2View::Store,
        DetailOrigin::Library => UiV2View::Library,
    }
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
    use super::{detail_back_destination, BrowseRequest, DetailOrigin, UiV2View};

    #[test]
    fn shell_does_not_use_generated_identity_or_fake_controls() {
        let source = include_str!("shell.rs");

        assert!(!source.contains(concat!("aida", "-public")));
        assert!(!source.contains(concat!("Za", "ps")));
        assert!(!source.contains(concat!("Notifica", "tions")));
        assert!(!source.contains(concat!("Sup", "port")));
    }

    #[test]
    fn every_shell_destination_has_mobile_navigation() {
        let source = include_str!("shell.rs");
        for label in [
            "Store",
            "Browse",
            "Library",
            "Community",
            "Publish",
            "Profile",
            "Achievements",
            "Purchases",
            "Settings",
        ] {
            assert!(
                source.contains(&format!("<MobileNavItem label=\"{label}\"")),
                "missing mobile navigation for {label}"
            );
        }
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
    let mobile_menu_open = RwSignal::new(false);
    let connected_relays = RwSignal::new(Vec::<String>::new());
    let relay_snapshot_loaded = RwSignal::new(false);
    let pending_relay_events = RwSignal::new(Vec::<(String, String)>::new());
    let allow_insecure_public_ws = RwSignal::new(false);
    let settings_error = RwSignal::new(None::<String>);

    Effect::new(move |_| {
        let connected_relays_for_effect = connected_relays;
        let relay_count_for_effect = relay_count;
        spawn_local(async move {
            // Subscribe first so early connection events are not missed.
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
                        // UiV2Root is app-lifetime; keep the JS callback alive for live relay events.
                        std::mem::forget(cleanup);
                    }
                    Err(err) => {
                        web_sys::console::error_1(
                            &format!("[UiV2Root] Failed to subscribe relay listener: {err}").into(),
                        );
                    }
                }
            }

            // Snapshot after subscribe to recover relays emitted before listener attachment.
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
    let mobile_menu_signal = Signal::derive(move || mobile_menu_open.get());
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

    let navigate_store = Callback::new(move |_| {
        current_view.set(UiV2View::Store);
        mobile_menu_open.set(false);
    });
    let navigate_browse = Callback::new(move |_| {
        current_view.set(UiV2View::Browse(BrowseRequest::default()));
        mobile_menu_open.set(false);
    });
    let search_browse = Callback::new(move |query: String| {
        current_view.set(UiV2View::Browse(BrowseRequest::for_query(query)));
        mobile_menu_open.set(false);
    });
    let navigate_store_category = Callback::new(move |request: BrowseRequest| {
        current_view.set(UiV2View::Browse(request));
        mobile_menu_open.set(false);
    });
    let navigate_library = Callback::new(move |_| {
        current_view.set(UiV2View::Library);
        mobile_menu_open.set(false);
    });
    let navigate_social = Callback::new(move |_| {
        current_view.set(UiV2View::Social);
        mobile_menu_open.set(false);
    });
    let navigate_publish = Callback::new(move |_| {
        current_view.set(UiV2View::Publish(PublishViewState::Games));
        mobile_menu_open.set(false);
    });
    let navigate_profile = Callback::new(move |_| {
        current_view.set(UiV2View::Profile);
        mobile_menu_open.set(false);
    });
    let navigate_achievements = Callback::new(move |_| {
        current_view.set(UiV2View::Achievements);
        mobile_menu_open.set(false);
    });
    let navigate_purchases = Callback::new(move |_| {
        current_view.set(UiV2View::Purchases);
        mobile_menu_open.set(false);
    });
    let navigate_settings = Callback::new(move |_| {
        current_view.set(UiV2View::Settings);
        mobile_menu_open.set(false);
    });
    let toggle_mobile_menu = Callback::new(move |_| {
        mobile_menu_open.update(|open| *open = !*open);
    });

    let on_select_listing = Callback::new(move |listing: GameListing| {
        current_view.set(UiV2View::Detail(listing, DetailOrigin::Store));
        mobile_menu_open.set(false);
    });
    let on_select_library_listing = Callback::new(move |listing: GameListing| {
        current_view.set(UiV2View::Detail(listing, DetailOrigin::Library));
        mobile_menu_open.set(false);
    });
    let on_back_from_detail = Callback::new(move |_| {
        let origin = match current_view.get_untracked() {
            UiV2View::Detail(_, origin) => origin,
            _ => DetailOrigin::Store,
        };
        current_view.set(detail_back_destination(origin));
    });
    let on_open_publish_from_profile = Callback::new(move |_| {
        current_view.set(UiV2View::Publish(PublishViewState::Games));
    });
    let on_open_listing_from_profile = Callback::new(move |listing: GameListing| {
        current_view.set(UiV2View::Detail(listing, DetailOrigin::Store));
    });
    let on_disconnect = Callback::new(move |_| {
        let auth_ctx = auth.clone();
        spawn_local(async move {
            match auth_ctx.logout_nip46().await {
                Ok(_) => auth_ctx.error.set(None),
                Err(err) => auth_ctx.error.set(Some(err)),
            }
        });
    });

    let store_active = Signal::derive(move || current_view.get() == UiV2View::Store);
    let browse_active = Signal::derive(move || {
        matches!(
            current_view.get(),
            UiV2View::Browse(_) | UiV2View::Detail(_, DetailOrigin::Store)
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
    let profile_active = Signal::derive(move || current_view.get() == UiV2View::Profile);
    let achievements_active = Signal::derive(move || current_view.get() == UiV2View::Achievements);
    let purchases_active = Signal::derive(move || current_view.get() == UiV2View::Purchases);
    let settings_active = Signal::derive(move || current_view.get() == UiV2View::Settings);

    #[cfg(target_arch = "wasm32")]
    {
        let previous_open = RwSignal::new(false);
        Effect::new(move |_| {
            let open = mobile_menu_open.get();
            let was_open = previous_open.get_untracked();
            previous_open.set(open);
            if open && !was_open {
                spawn_local(async move {
                    gloo_timers::future::TimeoutFuture::new(0).await;
                    let Some(document) = web_sys::window().and_then(|window| window.document())
                    else {
                        return;
                    };
                    let Some(nav) = document.get_element_by_id("mobile-primary-navigation") else {
                        return;
                    };
                    if let Ok(Some(first)) = nav.query_selector(MOBILE_NAV_FOCUSABLE) {
                        if let Ok(first) = first.dyn_into::<web_sys::HtmlElement>() {
                            let _ = first.focus();
                        }
                    }
                });
            } else if !open && was_open {
                if let Some(trigger) = web_sys::window()
                    .and_then(|window| window.document())
                    .and_then(|document| document.get_element_by_id("mobile-navigation-trigger"))
                    .and_then(|element| element.dyn_into::<web_sys::HtmlElement>().ok())
                {
                    let _ = trigger.focus();
                }
            }
        });
    }

    view! {
        <div class="min-h-screen bg-background text-on-surface selection:bg-primary/30">
            <style>{UI_V2_STYLES}</style>

            <TopBar
                relay_count=relay_count_signal
                connected_relays=connected_relays_signal
                display_name=display_name
                avatar_url=avatar_url
                avatar_fallback=avatar_fallback
                connection_status=connection_status
                connection_error=connection_error
                mobile_menu_open=mobile_menu_signal
                on_open_store=navigate_store
                on_open_browse=navigate_browse
                on_search=search_browse
                on_open_profile=navigate_profile
                on_toggle_mobile_menu=toggle_mobile_menu
            />

            <Show when=move || mobile_menu_open.get()>
                <button
                    type="button"
                    class="fixed inset-0 top-20 z-30 bg-black/55 lg:hidden"
                    aria-label="Close navigation"
                    on:click=move |_| mobile_menu_open.set(false)
                ></button>
                <nav
                    id="mobile-primary-navigation"
                    class="fixed inset-x-3 top-24 z-40 max-h-[calc(100vh-7rem)] overflow-auto rounded-2xl border border-white/10 bg-surface-container-low/95 p-3 shadow-ambient backdrop-blur-2xl lg:hidden"
                    aria-label="Primary navigation"
                    role="dialog"
                    aria-modal="true"
                    on:keydown=move |event| {
                        if event.key() == "Escape" {
                            event.prevent_default();
                            mobile_menu_open.set(false);
                        }
                        #[cfg(target_arch = "wasm32")]
                        if event.key() == "Tab" {
                            let Some(document) = web_sys::window().and_then(|window| window.document()) else { return };
                            let Some(nav) = document.get_element_by_id("mobile-primary-navigation") else { return };
                            let Ok(focusables) = nav.query_selector_all(MOBILE_NAV_FOCUSABLE) else { return };
                            let count = focusables.length();
                            if count == 0 { return; }
                            let first = focusables.item(0).and_then(|item| item.dyn_into::<web_sys::HtmlElement>().ok());
                            let last = focusables.item(count - 1).and_then(|item| item.dyn_into::<web_sys::HtmlElement>().ok());
                            let active = document.active_element();
                            let at_first = active.as_ref().zip(first.as_ref()).is_some_and(|(active, first)| active.is_same_node(Some(first.as_ref())));
                            let at_last = active.as_ref().zip(last.as_ref()).is_some_and(|(active, last)| active.is_same_node(Some(last.as_ref())));
                            if event.shift_key() && at_first {
                                event.prevent_default();
                                if let Some(last) = last { let _ = last.focus(); }
                            } else if !event.shift_key() && at_last {
                                event.prevent_default();
                                if let Some(first) = first { let _ = first.focus(); }
                            }
                        }
                    }
                >
                    <MobileNavItem label="Store" icon="grid_view" active=store_active on_click=navigate_store />
                    <MobileNavItem label="Browse" icon="explore" active=browse_active on_click=navigate_browse />
                    <MobileNavItem label="Library" icon="sports_esports" active=library_active on_click=navigate_library />
                    <MobileNavItem label="Community" icon="forum" active=social_active on_click=navigate_social />
                    <MobileNavItem label="Publish" icon="upload" active=publish_active on_click=navigate_publish />
                    <MobileNavItem label="Profile" icon="person" active=profile_active on_click=navigate_profile />
                    <MobileNavItem label="Achievements" icon="emoji_events" active=achievements_active on_click=navigate_achievements />
                    <MobileNavItem label="Purchases" icon="receipt_long" active=purchases_active on_click=navigate_purchases />
                    <MobileNavItem label="Settings" icon="settings" active=settings_active on_click=navigate_settings />
                </nav>
            </Show>

            <div class="mx-auto flex max-w-[1600px] items-start gap-6 px-4 py-6 md:px-8" inert=move || mobile_menu_open.get().then_some("")>
                <aside class="sticky top-24 hidden max-h-[calc(100vh-7rem)] w-60 shrink-0 flex-col overflow-auto rounded-2xl border border-white/5 bg-surface-container-low/75 p-4 shadow-ambient backdrop-blur-2xl lg:flex">
                    <nav class="flex flex-1 flex-col gap-1" aria-label="Primary navigation">
                        <NavItem label="Store" icon="grid_view" active=store_active on_click=navigate_store />
                        <NavItem label="Browse" icon="explore" active=browse_active on_click=navigate_browse />
                        <NavItem label="Library" icon="sports_esports" active=library_active on_click=navigate_library />
                        <NavItem label="Community" icon="forum" active=social_active on_click=navigate_social />
                        <NavItem label="Publish" icon="upload" active=publish_active on_click=navigate_publish />
                        <NavItem label="Profile" icon="person" active=profile_active on_click=navigate_profile />
                        <NavItem label="Achievements" icon="emoji_events" active=achievements_active on_click=navigate_achievements />
                        <NavItem label="Purchases" icon="receipt_long" active=purchases_active on_click=navigate_purchases />
                        <NavItem label="Settings" icon="settings" active=settings_active on_click=navigate_settings />
                    </nav>

                    <div class="mt-6 border-t border-white/5 pt-4">
                        <button
                            type="button"
                            class="flex w-full items-center gap-3 rounded-xl px-3 py-2.5 text-sm font-medium text-on-surface-variant outline-none ring-primary/60 hover:bg-surface-container-high/60 hover:text-on-surface focus-visible:ring-2"
                            on:click=move |_| on_disconnect.run(())
                        >
                            <span class="material-symbols-outlined text-lg" aria-hidden="true">"logout"</span>
                            <span>"Sign out"</span>
                        </button>
                    </div>
                </aside>

                <main class="min-w-0 flex-1">
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
                            <div class="p-4 md:p-8">
                                <GameDetailView listing=listing on_back=on_back_from_detail />
                            </div>
                        }
                        .into_any(),
                        UiV2View::Library => view! {
                            <div class="p-4 md:p-8">
                                <LibraryView on_open_listing=on_select_library_listing />
                            </div>
                        }
                        .into_any(),
                        UiV2View::Social => view! {
                            <div class="p-4 md:p-8"><SocialView /></div>
                        }
                        .into_any(),
                        UiV2View::Achievements => view! {
                            <div class="p-4 md:p-8"><AchievementsView /></div>
                        }
                        .into_any(),
                        UiV2View::Purchases => view! {
                            <div class="p-4 md:p-8"><PurchasesView /></div>
                        }
                        .into_any(),
                        UiV2View::Publish(state) => {
                            let on_navigate = Callback::new(move |state| {
                                current_view.set(UiV2View::Publish(state));
                            });
                            view! {
                                <div class="p-4 md:p-8">
                                    <PublishV2View state=state on_navigate=on_navigate />
                                </div>
                            }
                            .into_any()
                        }
                        UiV2View::Profile => view! {
                            <div class="p-4 md:p-8">
                                <ProfileV2View
                                    on_open_publish=on_open_publish_from_profile
                                    on_open_listing=on_open_listing_from_profile
                                />
                            </div>
                        }
                        .into_any(),
                        UiV2View::Settings => view! {
                            <section class="p-4 md:p-8">
                                <SettingsView
                                    connected_relays=connected_relays_signal
                                    allow_insecure_public_ws=allow_insecure_public_ws
                                    settings_error=settings_error
                                    on_sign_out=on_disconnect
                                />
                            </section>
                        }
                        .into_any(),
                    }}
                </main>
            </div>
        </div>
    }
}
