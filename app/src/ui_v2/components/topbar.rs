use leptos::prelude::*;

use super::transient::{close_transient_on_outside_pointer, close_transient_when_modal_opens};
use super::ArcadestrLogo;

#[cfg(target_arch = "wasm32")]
fn focus_topbar_control(id: &str) {
    use wasm_bindgen::JsCast;

    if let Some(element) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id(id))
        .and_then(|element| element.dyn_into::<web_sys::HtmlElement>().ok())
    {
        let _ = element.focus();
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn focus_topbar_control(_id: &str) {}

#[component]
pub fn TopBar(
    relay_count: Signal<usize>,
    connected_relays: Signal<Vec<String>>,
    display_name: Signal<String>,
    avatar_url: Signal<Option<String>>,
    avatar_fallback: Signal<String>,
    connection_status: Signal<String>,
    connection_error: Signal<Option<String>>,
    discover_active: Signal<bool>,
    library_active: Signal<bool>,
    community_active: Signal<bool>,
    publish_active: Signal<bool>,
    on_open_store: Callback<()>,
    on_open_browse: Callback<()>,
    on_search: Callback<String>,
    on_open_library: Callback<()>,
    on_open_community: Callback<()>,
    on_open_publish: Callback<()>,
    on_open_profile: Callback<()>,
    on_open_achievements: Callback<()>,
    on_open_purchases: Callback<()>,
    on_open_settings: Callback<()>,
    on_disconnect: Callback<()>,
) -> impl IntoView {
    let relay_menu_open = RwSignal::new(false);
    let account_menu_open = RwSignal::new(false);

    // Shared transient contract: an outside pointer press dismisses, and a
    // modal opening dismisses. Escape and the close control already dismiss and
    // restore focus to the trigger below.
    close_transient_on_outside_pointer(relay_menu_open, "#relay-menu-wrap");
    close_transient_on_outside_pointer(account_menu_open, "#account-menu-wrap");
    close_transient_when_modal_opens(relay_menu_open);
    close_transient_when_modal_opens(account_menu_open);
    let search_query = RwSignal::new(String::new());

    let close_account_and = move |action: Callback<()>| {
        account_menu_open.set(false);
        focus_topbar_control("account-control-trigger");
        action.run(());
    };

    view! {
        <header class="arc-topbar">
            <div class="arc-topbar-inner">
                <ArcadestrLogo on_click=on_open_store />

                <nav class="arc-primary-nav" aria-label="Primary navigation">
                    <button
                        type="button"
                        class:arc-nav-active=move || discover_active.get()
                        aria-current=move || discover_active.get().then_some("page")
                        on:click=move |_| on_open_store.run(())
                    >
                        "Discover"
                    </button>
                    <button
                        type="button"
                        class:arc-nav-active=move || library_active.get()
                        aria-current=move || library_active.get().then_some("page")
                        on:click=move |_| on_open_library.run(())
                    >
                        "Library"
                    </button>
                    <button
                        type="button"
                        class:arc-nav-active=move || community_active.get()
                        aria-current=move || community_active.get().then_some("page")
                        on:click=move |_| on_open_community.run(())
                    >
                        "Community"
                    </button>
                    <button
                        type="button"
                        class:arc-nav-active=move || publish_active.get()
                        aria-current=move || publish_active.get().then_some("page")
                        on:click=move |_| on_open_publish.run(())
                    >
                        "Publish"
                    </button>
                </nav>

                <div class="arc-topbar-actions">
                    <form
                        class="arc-search"
                        role="search"
                        on:submit=move |event| {
                            event.prevent_default();
                            let query = search_query.get_untracked().trim().to_string();
                            if query.is_empty() {
                                on_open_browse.run(());
                            } else {
                                on_search.run(query);
                            }
                        }
                    >
                        <label>
                            <span class="material-symbols-outlined" aria-hidden="true">"search"</span>
                            <span class="sr-only">"Search games, creators, and tags"</span>
                            <input
                                type="search"
                                placeholder="Search games, creators, tags"
                                prop:value=move || search_query.get()
                                on:input=move |event| search_query.set(event_target_value(&event))
                            />
                        </label>
                    </form>

                    <div id="relay-menu-wrap" class="arc-topbar-menu-wrap">
                        <button
                            id="relay-control-trigger"
                            type="button"
                            class="arc-relay-control"
                            aria-label="Show relay connections"
                            aria-expanded=move || relay_menu_open.get()
                            aria-controls="relay-status-menu"
                            title=move || match relay_count.get() {
                                0 => "No connected relays".to_string(),
                                1 => "1 connected relay".to_string(),
                                count => format!("{count} connected relays"),
                            }
                            on:click=move |_| {
                                account_menu_open.set(false);
                                relay_menu_open.update(|open| *open = !*open);
                            }
                            on:keydown=move |event| {
                                if event.key() == "Escape" && relay_menu_open.get_untracked() {
                                    event.prevent_default();
                                    relay_menu_open.set(false);
                                }
                            }
                        >
                            <span
                                class="arc-relay-dot"
                                class:arc-relay-dot-offline=move || relay_count.get() == 0
                                aria-hidden="true"
                            ></span>
                            <span>
                                {move || match relay_count.get() {
                                    0 => "NO RELAYS".to_string(),
                                    1 => "1 RELAY".to_string(),
                                    count => format!("{count} RELAYS"),
                                }}
                            </span>
                        </button>

                        <Show when=move || relay_menu_open.get()>
                            <section
                                id="relay-status-menu"
                                class="arc-topbar-menu arc-relay-menu"
                                aria-label="Relay connections"
                                on:keydown=move |event| {
                                    if event.key() == "Escape" {
                                        event.prevent_default();
                                        relay_menu_open.set(false);
                                        focus_topbar_control("relay-control-trigger");
                                    }
                                }
                            >
                                <div class="arc-menu-heading">
                                    <h2>"Relay connections"</h2>
                                    <button
                                        type="button"
                                        class="arc-menu-close"
                                        aria-label="Close relay connections"
                                        on:click=move |_| {
                                            relay_menu_open.set(false);
                                            focus_topbar_control("relay-control-trigger");
                                        }
                                    >
                                        <span class="material-symbols-outlined" aria-hidden="true">"close"</span>
                                    </button>
                                </div>
                                {move || {
                                    let relays = connected_relays.get();
                                    if relays.is_empty() {
                                        view! { <p class="arc-menu-empty">"No relays are currently connected."</p> }
                                            .into_any()
                                    } else {
                                        view! {
                                            <ul class="arc-relay-list">
                                                {relays
                                                    .into_iter()
                                                    .map(|relay| view! {
                                                        <li>
                                                            <span class="arc-relay-dot" aria-hidden="true"></span>
                                                            <span>{relay}</span>
                                                        </li>
                                                    })
                                                    .collect::<Vec<_>>()}
                                            </ul>
                                        }
                                        .into_any()
                                    }
                                }}
                                <button
                                    type="button"
                                    class="arc-menu-link"
                                    on:click=move |_| {
                                        relay_menu_open.set(false);
                                        focus_topbar_control("relay-control-trigger");
                                        on_open_settings.run(());
                                    }
                                >
                                    "Network settings"
                                </button>
                            </section>
                        </Show>
                    </div>

                    <div id="account-menu-wrap" class="arc-topbar-menu-wrap">
                        <button
                            id="account-control-trigger"
                            type="button"
                            class="arc-account-control"
                            aria-label=move || format!("Open account menu for {}", display_name.get())
                            aria-expanded=move || account_menu_open.get()
                            aria-controls="account-control-menu"
                            title=move || {
                                let status = connection_status.get();
                                if let Some(error) = connection_error.get() {
                                    format!("Signer: {status} ({error})")
                                } else {
                                    format!("Signer: {status}")
                                }
                            }
                            on:click=move |_| {
                                relay_menu_open.set(false);
                                account_menu_open.update(|open| *open = !*open);
                            }
                            on:keydown=move |event| {
                                if event.key() == "Escape" && account_menu_open.get_untracked() {
                                    event.prevent_default();
                                    account_menu_open.set(false);
                                }
                            }
                        >
                            {move || match avatar_url.get() {
                                Some(url) => view! {
                                    <img src=url alt="" class="arc-account-avatar" />
                                }
                                .into_any(),
                                None => view! {
                                    <span class="arc-account-avatar arc-account-fallback" aria-hidden="true">
                                        {move || avatar_fallback.get()}
                                    </span>
                                }
                                .into_any(),
                            }}
                            <span class="arc-account-name">{move || display_name.get()}</span>
                            <span class="material-symbols-outlined arc-account-chevron" aria-hidden="true">"expand_more"</span>
                        </button>

                        <Show when=move || account_menu_open.get()>
                            <nav
                                id="account-control-menu"
                                class="arc-topbar-menu arc-account-menu"
                                aria-label="Account navigation"
                                on:keydown=move |event| {
                                    if event.key() == "Escape" {
                                        event.prevent_default();
                                        account_menu_open.set(false);
                                        focus_topbar_control("account-control-trigger");
                                    }
                                }
                            >
                                <p class="arc-signer-state">
                                    <span>"Signer"</span>
                                    <strong>{move || connection_status.get()}</strong>
                                </p>
                                <button type="button" on:click=move |_| close_account_and(on_open_profile)>
                                    "View profile"
                                </button>
                                <button type="button" on:click=move |_| close_account_and(on_open_achievements)>
                                    "Achievements"
                                </button>
                                <button type="button" on:click=move |_| close_account_and(on_open_purchases)>
                                    "Purchases"
                                </button>
                                <button type="button" on:click=move |_| close_account_and(on_open_settings)>
                                    "Accounts & settings"
                                </button>
                                <button
                                    type="button"
                                    class="arc-menu-signout"
                                    on:click=move |_| close_account_and(on_disconnect)
                                >
                                    "Sign out"
                                </button>
                            </nav>
                        </Show>
                    </div>
                </div>
            </div>
        </header>
    }
}
