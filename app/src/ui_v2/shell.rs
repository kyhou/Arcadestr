use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::{closure::Closure, JsCast, JsValue};

use crate::models::GameListing;
use crate::ui_v2::components::{NavItem, TopBar};
use crate::ui_v2::theme::UI_V2_STYLES;
use crate::ui_v2::views::{
    BrowseGamesView, GameDetailView, LibraryView, ProfileV2View, PublishV2View, SocialView,
    StoreFrontView,
};
use crate::{invoke_get_connected_relays, invoke_logout_nip46, AuthContext};

#[derive(Clone, PartialEq)]
enum UiV2View {
    Store,
    BrowseAll,
    Detail(GameListing),
    Library,
    Social,
    Publish,
    Profile,
    Settings,
}

#[component]
pub fn UiV2Root(relay_count: RwSignal<usize>) -> impl IntoView {
    let auth = use_context::<AuthContext>().expect("AuthContext not provided");
    let current_view = RwSignal::new(UiV2View::Store);
    let connected_relays = RwSignal::new(Vec::<String>::new());

    Effect::new(move |_| {
        let connected_relays_for_effect = connected_relays.clone();
        let relay_count_for_effect = relay_count;
        spawn_local(async move {
            if let Ok(relays) = invoke_get_connected_relays().await {
                relay_count_for_effect.set(relays.len());
                connected_relays_for_effect.set(relays);
            }

            #[cfg(target_arch = "wasm32")]
            {
                let window = web_sys::window().expect("window should be available");
                if let Ok(tauri) = js_sys::Reflect::get(&window, &"__TAURI__".into()) {
                    if let Ok(event_api) = js_sys::Reflect::get(&tauri, &"event".into()) {
                        let relays_for_listener = connected_relays_for_effect.clone();
                        let relay_count_for_listener = relay_count_for_effect;

                        let closure = Closure::wrap(Box::new(move |event: JsValue| {
                            let payload = match js_sys::Reflect::get(&event, &"payload".into()) {
                                Ok(p) => p,
                                Err(_) => return,
                            };

                            let event_type = js_sys::Reflect::get(&payload, &"type".into())
                                .ok()
                                .and_then(|v| v.as_string())
                                .unwrap_or_default();
                            let url = js_sys::Reflect::get(&payload, &"url".into())
                                .ok()
                                .and_then(|v| v.as_string())
                                .unwrap_or_default();

                            match event_type.as_str() {
                                "connected" => {
                                    relays_for_listener.update(|relays| {
                                        if !relays.contains(&url) {
                                            relays.push(url.clone());
                                        }
                                    });
                                }
                                "disconnected" => {
                                    relays_for_listener.update(|relays| {
                                        relays.retain(|relay| relay != &url);
                                    });
                                }
                                _ => {}
                            }

                            relay_count_for_listener.set(relays_for_listener.get_untracked().len());
                        })
                            as Box<dyn FnMut(JsValue)>);

                        if let Ok(listen) = js_sys::Reflect::get(&event_api, &"listen".into()) {
                            if let Some(listen_fn) = listen.dyn_ref::<js_sys::Function>() {
                                let _ = listen_fn.call2(
                                    &event_api,
                                    &"relay-connection".into(),
                                    &closure.as_ref().into(),
                                );
                                closure.forget();
                            }
                        }
                    }
                }
            }
        });
    });

    let relay_count_signal = Signal::derive(move || relay_count.get());
    let connected_relays_signal = Signal::derive(move || connected_relays.get());
    let connection_status = Signal::derive(move || auth.connection_status.get());
    let connection_error = Signal::derive(move || auth.connection_error.get());
    let search_placeholder = Signal::derive(move || match current_view.get() {
        UiV2View::Store => "Search curated games...",
        UiV2View::BrowseAll => "Search games, developers, notes...",
        UiV2View::Detail(_) => "Search curated worlds...",
        UiV2View::Library => "Search your library...",
        UiV2View::Social => "Search the protocol...",
        _ => "Search games...",
    });
    let browse_active = Signal::derive(move || {
        matches!(
            current_view.get(),
            UiV2View::BrowseAll | UiV2View::Detail(_)
        )
    });
    let display_name = Signal::derive(move || {
        auth.profile
            .get()
            .map(|profile| profile.display())
            .or_else(|| auth.npub.get())
            .unwrap_or_else(|| "Neon Curator".to_string())
    });

    let set_store = move |_| current_view.set(UiV2View::Store);
    let set_browse_all = move |_| current_view.set(UiV2View::BrowseAll);
    let set_library = move |_| current_view.set(UiV2View::Library);
    let set_social = move |_| current_view.set(UiV2View::Social);
    let set_publish = move |_| current_view.set(UiV2View::Publish);
    let set_profile = move |_| current_view.set(UiV2View::Profile);
    let set_settings = move |_| current_view.set(UiV2View::Settings);
    let on_select_listing = Callback::new(move |listing: GameListing| {
        current_view.set(UiV2View::Detail(listing));
    });
    let on_open_browse_all = Callback::new(move |_| {
        current_view.set(UiV2View::BrowseAll);
    });
    let on_back_to_store = Callback::new(move |_| {
        current_view.set(UiV2View::Store);
    });
    let on_open_publish_from_profile = Callback::new(move |_| {
        current_view.set(UiV2View::Publish);
    });
    let on_open_listing_from_profile = Callback::new(move |listing: GameListing| {
        current_view.set(UiV2View::Detail(listing));
    });
    let on_disconnect = Callback::new(move |_| {
        let auth_ctx = auth.clone();
        spawn_local(async move {
            match invoke_logout_nip46().await {
                Ok(_) => {
                    auth_ctx.npub.set(None);
                    auth_ctx.error.set(None);
                    auth_ctx.active_account.set(None);
                    auth_ctx.connection_status.set("disconnected".to_string());
                    auth_ctx.connection_error.set(None);
                    let _ = auth_ctx.load_accounts_list().await;
                }
                Err(err) => auth_ctx.error.set(Some(err)),
            }
        });
    });

    view! {
        <div class="min-h-screen bg-background text-on-surface selection:bg-primary/30">
            <style>{UI_V2_STYLES}</style>

            <aside
                class="fixed left-0 top-20 h-[calc(100vh-5rem)] w-64 z-40 bg-[#0f141a]/60 backdrop-blur-2xl border-r shadow-[20px_0px_40px_rgba(0,0,0,0.4)] flex flex-col py-6 gap-2 hidden md:flex"
                style="border-right-color: rgba(68, 72, 79, 0.15);"
            >
                <div class="px-6 mb-8">
                    <div class="flex items-center gap-3">
                        <img alt="Nostr Avatar" class="w-10 h-10 rounded-full bg-surface-container-high" src="https://lh3.googleusercontent.com/aida-public/AB6AXuDonh-oj27BASn7hRbc5ABl3sahWXPvHlPRriWjdt6XOn9NTuz3Yhov4Y5n3D2N3gv5ZAYxmNJAhPcMkdwqI0RF7FPMPzC2XYgVPsbydtgmvV47MYyDu7gxbEhpZkg4FplYMwJL7fUsav2O363fF9z5kGa4GY4p53YZpIlVd1pGzL9qKI5cwYcWoamMHRzH5IlEg3Yaxa3bMR52OanALmy4DlzubsGLV022a7sIH-m6tG77PfALprgA-sNVjvbU9siIrnoYktG5qYI" />
                        <div>
                            <h3 class="font-bold text-on-surface text-sm">{move || display_name.get()}</h3>
                            <p class="text-tertiary text-xs">"⚡ 4.2k Zaps"</p>
                        </div>
                    </div>
                </div>

                <nav class="flex-1">
                    <NavItem
                        label="Store"
                        icon="grid_view"
                        active={Signal::derive(move || {
                            matches!(current_view.get(), UiV2View::Store | UiV2View::BrowseAll)
                        })}
                        on_click={Callback::new(set_store)}
                    />
                    <NavItem
                        label="Library"
                        icon="sports_esports"
                        active={Signal::derive(move || current_view.get() == UiV2View::Library)}
                        on_click={Callback::new(set_library)}
                    />
                    <NavItem
                        label="Social"
                        icon="forum"
                        active={Signal::derive(move || current_view.get() == UiV2View::Social)}
                        on_click={Callback::new(set_social)}
                    />
                    <NavItem
                        label="Publish"
                        icon="upload"
                        active={Signal::derive(move || current_view.get() == UiV2View::Publish)}
                        on_click={Callback::new(set_publish)}
                    />
                    <NavItem
                        label="Profile"
                        icon="person"
                        active={Signal::derive(move || current_view.get() == UiV2View::Profile)}
                        on_click={Callback::new(set_profile)}
                    />
                    <NavItem
                        label="Settings"
                        icon="settings"
                        active={Signal::derive(move || current_view.get() == UiV2View::Settings)}
                        on_click={Callback::new(set_settings)}
                    />
                </nav>

                <div class="px-4 mb-4">
                    <button class="w-full bg-gradient-to-r from-primary to-primary-dim text-on-primary font-bold py-3 rounded-md active:scale-95 transition-all text-sm">
                        "Connect Nostr"
                    </button>
                </div>

                <div class="border-t border-outline-variant/10 pt-4">
                    <button class="flex items-center gap-4 text-[#f1f3fc]/50 px-4 py-3 mx-2 my-1 hover:bg-[#262c36]/30 cursor-pointer active:opacity-80 transition-transform duration-200 hover:translate-x-1 font-['Inter'] text-sm font-medium w-[calc(100%-1rem)] rounded-lg text-left">
                        <span class="material-symbols-outlined">"help_outline"</span>
                        <span>"Support"</span>
                    </button>
                    <button
                        class="flex items-center gap-4 text-[#f1f3fc]/50 px-4 py-3 mx-2 my-1 hover:bg-[#262c36]/30 cursor-pointer active:opacity-80 transition-transform duration-200 hover:translate-x-1 font-['Inter'] text-sm font-medium w-[calc(100%-1rem)] rounded-lg text-left"
                        on:click=move |_| on_disconnect.run(())
                    >
                        <span class="material-symbols-outlined">"logout"</span>
                        <span>"Sign Out"</span>
                    </button>
                </div>
            </aside>

            <TopBar
                relay_count={relay_count_signal}
                connected_relays={connected_relays_signal}
                display_name={display_name}
                connection_status={connection_status}
                connection_error={connection_error}
                on_open_profile={Callback::new(set_profile)}
                search_placeholder={search_placeholder.get()}
                browse_active={browse_active.get()}
            />

            <main class="md:pl-64 pt-20 min-h-screen pb-24 md:pb-0">
                {move || {
                    match current_view.get() {
                        UiV2View::Store => {
                            view! {
                                <StoreFrontView
                                    on_select={on_select_listing}
                                    on_view_all={on_open_browse_all}
                                />
                            }
                                .into_any()
                        }
                        UiV2View::BrowseAll => {
                            view! { <BrowseGamesView on_select={on_select_listing} /> }
                                .into_any()
                        }
                        UiV2View::Detail(listing) => {
                            view! {
                                <div class="max-w-[1600px] mx-auto p-8">
                                    <GameDetailView listing={listing} on_back={on_back_to_store} />
                                </div>
                            }
                            .into_any()
                        }
                        UiV2View::Library => {
                            view! { <div class="max-w-[1600px] mx-auto p-8"><LibraryView /></div> }
                                .into_any()
                        }
                        UiV2View::Social => {
                            view! { <div class="max-w-[1600px] mx-auto p-8"><SocialView /></div> }
                                .into_any()
                        }
                        UiV2View::Publish => {
                            view! { <div class="max-w-[1600px] mx-auto p-8"><PublishV2View /></div> }
                                .into_any()
                        }
                        UiV2View::Profile => view! {
                            <div class="max-w-[1600px] mx-auto p-8">
                                <ProfileV2View
                                    on_open_publish={on_open_publish_from_profile}
                                    on_open_listing={on_open_listing_from_profile}
                                />
                            </div>
                        }
                        .into_any(),
                        UiV2View::Settings => {
                            view! {
                                <section class="max-w-[1600px] mx-auto p-8">
                                    <div class="bg-surface-container-high rounded-xl p-6">
                                        <h2 class="font-headline text-3xl font-bold tracking-tight">"Settings"</h2>
                                        <p class="text-on-surface-variant mt-2">"Account preferences and client options will land in the next parity pass."</p>
                                    </div>
                                </section>
                            }
                            .into_any()
                        }
                    }
                }}
            </main>

            <nav class="md:hidden fixed bottom-0 left-0 right-0 h-16 bg-surface-container-low/90 backdrop-blur-xl border-t border-outline-variant/10 px-6 flex justify-between items-center z-50">
                <button class="flex flex-col items-center text-primary" on:click=move |_| current_view.set(UiV2View::Store)>
                    <span class="material-symbols-outlined">"grid_view"</span>
                    <span class="text-[10px] font-bold">"Store"</span>
                </button>
                <button class="flex flex-col items-center text-on-surface-variant" on:click=move |_| set_browse_all(())>
                    <span class="material-symbols-outlined">"explore"</span>
                    <span class="text-[10px] font-medium">"Browse"</span>
                </button>
                <button class="flex flex-col items-center text-on-surface-variant" on:click=move |_| current_view.set(UiV2View::Library)>
                    <span class="material-symbols-outlined">"sports_esports"</span>
                    <span class="text-[10px] font-medium">"Library"</span>
                </button>
                <button class="flex flex-col items-center text-on-surface-variant" on:click=move |_| current_view.set(UiV2View::Social)>
                    <span class="material-symbols-outlined">"forum"</span>
                    <span class="text-[10px] font-medium">"Social"</span>
                </button>
                <button class="flex flex-col items-center text-on-surface-variant" on:click=move |_| current_view.set(UiV2View::Settings)>
                    <span class="material-symbols-outlined">"settings"</span>
                    <span class="text-[10px] font-medium">"Settings"</span>
                </button>
            </nav>
        </div>
    }
}
