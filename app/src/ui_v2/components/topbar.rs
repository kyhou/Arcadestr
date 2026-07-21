use leptos::prelude::*;

#[component]
pub fn TopBar(
    page_title: Signal<&'static str>,
    relay_count: Signal<usize>,
    connected_relays: Signal<Vec<String>>,
    display_name: Signal<String>,
    avatar_url: Signal<Option<String>>,
    avatar_fallback: Signal<String>,
    connection_status: Signal<String>,
    connection_error: Signal<Option<String>>,
    mobile_menu_open: Signal<bool>,
    on_open_store: Callback<()>,
    on_open_profile: Callback<()>,
    on_toggle_mobile_menu: Callback<()>,
) -> impl IntoView {
    let relay_menu_open = RwSignal::new(false);

    view! {
        <header class="sticky top-0 z-50 border-b border-white/5 bg-background/75 backdrop-blur-2xl">
            <div class="mx-auto flex h-20 max-w-[1600px] items-center gap-3 px-4 md:px-8">
                <button
                    type="button"
                    class="font-display text-2xl font-bold text-transparent outline-none ring-primary/60 [background:var(--noir-gradient-primary)] bg-clip-text focus-visible:ring-2"
                    aria-label="Go to Store"
                    on:click=move |_| on_open_store.run(())
                >
                    "Arcadestr"
                </button>

                <div class="hidden h-5 w-px bg-white/10 sm:block" aria-hidden="true"></div>
                <p class="hidden truncate text-sm font-medium text-on-surface-variant sm:block">
                    {move || page_title.get()}
                </p>

                <div class="ml-auto flex items-center gap-2 sm:gap-3">
                    <div class="relative">
                        <button
                            type="button"
                            class="flex min-h-10 items-center gap-2 rounded-full bg-surface-container-high px-3 py-2 text-xs font-semibold text-on-surface outline-none ring-primary/60 hover:bg-surface-bright focus-visible:ring-2"
                            aria-label="Show relay connections"
                            aria-expanded=move || relay_menu_open.get()
                            aria-controls="relay-status-menu"
                            title={move || if relay_count.get() > 0 {
                                format!("{} connected relays", relay_count.get())
                            } else {
                                "No connected relays".to_string()
                            }}
                            on:click=move |_| relay_menu_open.update(|open| *open = !*open)
                        >
                            <span
                                class={move || if relay_count.get() > 0 {
                                    "h-2 w-2 rounded-full bg-emerald-400"
                                } else {
                                    "h-2 w-2 rounded-full bg-on-surface-variant/50"
                                }}
                                aria-hidden="true"
                            ></span>
                            <span class="hidden uppercase tracking-widest sm:inline">
                                {move || match relay_count.get() {
                                    0 => "No relays".to_string(),
                                    1 => "1 relay".to_string(),
                                    count => format!("{count} relays"),
                                }}
                            </span>
                            <span class="sm:hidden">{move || relay_count.get()}</span>
                        </button>

                        <Show when=move || relay_menu_open.get()>
                            <section
                                id="relay-status-menu"
                                class="absolute right-0 top-12 z-[70] max-h-72 w-[min(20rem,calc(100vw-2rem))] overflow-auto rounded-2xl border border-white/10 bg-surface-container-high/95 p-4 shadow-ambient backdrop-blur-2xl"
                                aria-label="Relay connections"
                                on:keydown=move |event| {
                                    if event.key() == "Escape" {
                                        relay_menu_open.set(false);
                                    }
                                }
                            >
                                <div class="mb-3 flex items-center justify-between gap-3">
                                    <h2 class="font-display text-sm font-semibold">"Relay connections"</h2>
                                    <button
                                        type="button"
                                        class="rounded-lg px-2 py-1 text-xs text-on-surface-variant outline-none ring-primary/60 hover:text-on-surface focus-visible:ring-2"
                                        on:click=move |_| relay_menu_open.set(false)
                                    >
                                        "Close"
                                    </button>
                                </div>
                                {move || {
                                    let relays = connected_relays.get();
                                    if relays.is_empty() {
                                        view! {
                                            <p class="text-sm text-on-surface-variant">
                                                "No relays are currently connected."
                                            </p>
                                        }
                                        .into_any()
                                    } else {
                                        view! {
                                            <ul class="space-y-2">
                                                {relays
                                                    .into_iter()
                                                    .map(|relay| view! {
                                                        <li class="flex items-center gap-2 break-all rounded-xl bg-surface-container-low px-3 py-2 text-xs text-on-surface-variant">
                                                            <span class="h-1.5 w-1.5 shrink-0 rounded-full bg-emerald-400" aria-hidden="true"></span>
                                                            {relay}
                                                        </li>
                                                    })
                                                    .collect::<Vec<_>>()}
                                            </ul>
                                        }
                                        .into_any()
                                    }
                                }}
                            </section>
                        </Show>
                    </div>

                    <button
                        id="mobile-navigation-trigger"
                        type="button"
                        class="flex min-h-10 items-center gap-2 rounded-full p-1.5 pr-2 text-left outline-none ring-primary/60 hover:bg-surface-container-high focus-visible:ring-2"
                        aria-label=move || format!("Open profile for {}", display_name.get())
                        title=move || {
                            let status = connection_status.get();
                            if let Some(error) = connection_error.get() {
                                format!("Signer: {status} ({error})")
                            } else {
                                format!("Signer: {status}")
                            }
                        }
                        on:click=move |_| on_open_profile.run(())
                    >
                        {move || match avatar_url.get() {
                            Some(url) => view! {
                                <img
                                    src=url
                                    alt=""
                                    class="h-8 w-8 rounded-full object-cover ring-1 ring-primary/50"
                                />
                            }
                            .into_any(),
                            None => view! {
                                <span class="flex h-8 w-8 items-center justify-center rounded-full bg-surface-bright text-xs font-bold text-on-surface ring-1 ring-primary/50" aria-hidden="true">
                                    {move || avatar_fallback.get()}
                                </span>
                            }
                            .into_any(),
                        }}
                        <span class="hidden max-w-32 truncate text-xs font-semibold text-on-surface lg:block">
                            {move || display_name.get()}
                        </span>
                    </button>

                    <button
                        type="button"
                        class="flex h-10 w-10 items-center justify-center rounded-xl text-on-surface-variant outline-none ring-primary/60 hover:bg-surface-container-high hover:text-on-surface focus-visible:ring-2 md:hidden"
                        aria-label={move || if mobile_menu_open.get() { "Close navigation" } else { "Open navigation" }}
                        aria-expanded=move || mobile_menu_open.get()
                        aria-controls="mobile-primary-navigation"
                        on:click=move |_| on_toggle_mobile_menu.run(())
                    >
                        <span class="material-symbols-outlined" aria-hidden="true">
                            {move || if mobile_menu_open.get() { "close" } else { "menu" }}
                        </span>
                    </button>
                </div>
            </div>
        </header>
    }
}
