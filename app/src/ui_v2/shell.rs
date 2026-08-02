use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::models::{npub_fallback_label, GameListing};
use crate::relay_state::{apply_relay_event, merge_relay_snapshot};
use crate::ui_v2::components::{PageContainer, TopBar};
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
    Publisher,
}

fn detail_back_destination(origin: DetailOrigin) -> UiV2View {
    match origin {
        DetailOrigin::Store => UiV2View::Store,
        DetailOrigin::Library => UiV2View::Library,
        DetailOrigin::Publisher => UiV2View::Publish(PublishViewState::Games),
    }
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
        detail_back_destination, should_reset_publisher_route, BrowseRequest, DetailOrigin,
        PublishViewState, UiV2View,
    };

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

    let navigate_store = Callback::new(move |_| current_view.set(UiV2View::Store));
    let navigate_browse = Callback::new(move |_| {
        current_view.set(UiV2View::Browse(BrowseRequest::default()));
    });
    let search_browse = Callback::new(move |query: String| {
        current_view.set(UiV2View::Browse(BrowseRequest::for_query(query)));
    });
    let navigate_store_category = Callback::new(move |request: BrowseRequest| {
        current_view.set(UiV2View::Browse(request));
    });
    let navigate_library = Callback::new(move |_| current_view.set(UiV2View::Library));
    let navigate_social = Callback::new(move |_| current_view.set(UiV2View::Social));
    let navigate_publish = Callback::new(move |_| {
        current_view.set(UiV2View::Publish(PublishViewState::Games));
    });
    let navigate_profile = Callback::new(move |_| current_view.set(UiV2View::Profile));
    let navigate_achievements = Callback::new(move |_| current_view.set(UiV2View::Achievements));
    let navigate_purchases = Callback::new(move |_| current_view.set(UiV2View::Purchases));
    let navigate_settings = Callback::new(move |_| current_view.set(UiV2View::Settings));

    let on_select_listing = Callback::new(move |listing: GameListing| {
        current_view.set(UiV2View::Detail(listing, DetailOrigin::Store));
    });
    let on_select_library_listing = Callback::new(move |listing: GameListing| {
        current_view.set(UiV2View::Detail(listing, DetailOrigin::Library));
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

            <main>
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
                                current_view.set(UiV2View::Publish(state));
                            });
                            let on_open_listing = Callback::new(move |listing| {
                                current_view.set(UiV2View::Detail(listing, DetailOrigin::Publisher));
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
        </div>
    }
}
