use leptos::prelude::*;

#[component]
pub fn TopBar(
    relay_count: Signal<usize>,
    connected_relays: Signal<Vec<String>>,
    display_name: Signal<String>,
    connection_status: Signal<String>,
    connection_error: Signal<Option<String>>,
    on_open_profile: Callback<()>,
    #[prop(optional)] search_placeholder: Option<&'static str>,
    #[prop(optional)] browse_active: Option<bool>,
) -> impl IntoView {
    let placeholder = search_placeholder.unwrap_or("Search games...");
    let browse_active = browse_active.unwrap_or(false);
    let relay_menu_open = RwSignal::new(false);

    view! {
        <header class="fixed top-0 w-full z-50 bg-[#0a0e14]/80 backdrop-blur-xl flex justify-between items-center h-20 px-8 bg-gradient-to-b from-[#0a0e14] to-transparent">
            <div class="flex items-center gap-12">
                <span class="text-2xl font-bold bg-gradient-to-r from-[#b6a0ff] to-[#7e51ff] bg-clip-text text-transparent font-headline tracking-tight">"Arcadestr"</span>
                <nav class="hidden md:flex gap-8 items-center">
                    <a
                        class={if browse_active {
                            "text-[#f1f3fc]/60 hover:text-[#f1f3fc] transition-colors font-['Space_Grotesk'] tracking-tight"
                        } else {
                            "text-[#b6a0ff] font-bold border-b-2 border-[#b6a0ff] pb-1 font-['Space_Grotesk'] tracking-tight"
                        }}
                        href="#"
                    >
                        "Discover"
                    </a>
                    <a
                        class={if browse_active {
                            "text-[#b6a0ff] font-bold border-b-2 border-[#b6a0ff] pb-1 font-['Space_Grotesk'] tracking-tight"
                        } else {
                            "text-[#f1f3fc]/60 hover:text-[#f1f3fc] transition-colors font-['Space_Grotesk'] tracking-tight"
                        }}
                        href="#"
                    >
                        "Browse"
                    </a>
                </nav>
            </div>

            <div class="flex items-center gap-6 relative">
                <button
                    class="hidden sm:flex items-center gap-2 px-3 py-1.5 bg-surface-container-high/40 rounded-full border border-outline-variant/20"
                    title={move || {
                        let status = connection_status.get();
                        if let Some(err) = connection_error.get() {
                            format!("{} ({})", status, err)
                        } else {
                            status
                        }
                    }}
                    on:click=move |_| relay_menu_open.update(|open| *open = !*open)
                >
                    <span class="flex h-2 w-2 relative">
                        <span class="animate-ping absolute inline-flex h-full w-full rounded-full bg-emerald-400 opacity-75"></span>
                        <span class="relative inline-flex rounded-full h-2 w-2 bg-emerald-500"></span>
                    </span>
                    <span class="text-[10px] font-bold tracking-tight text-on-surface/80 font-['Space_Grotesk'] uppercase">{move || relay_count.get().to_string()}</span>
                </button>

                <Show when=move || crate::debug_storefront_bypass_enabled()>
                    <span class="hidden md:inline-flex items-center px-2.5 py-1 rounded-full text-[10px] font-bold tracking-widest uppercase bg-warning/25 text-warning border border-warning/30">
                        "Debug Mock Mode"
                    </span>
                </Show>

                <Show when=move || relay_menu_open.get()>
                    <div class="absolute top-14 right-48 min-w-80 max-h-72 overflow-auto p-3 z-[100] bg-surface-container-high/90 border border-outline-variant/40 rounded-xl backdrop-blur-xl">
                        <div class="flex items-center justify-between mb-2">
                            <strong class="text-sm">"Connected relays"</strong>
                            <button class="text-xs text-on-surface-variant hover:text-on-surface" on:click=move |_| relay_menu_open.set(false)>"Close"</button>
                        </div>
                        {move || {
                            let relays = connected_relays.get();
                            if relays.is_empty() {
                                view! { <p class="text-xs text-on-surface-variant">"No relays connected"</p> }.into_any()
                            } else {
                                view! {
                                    <ul class="space-y-1">
                                        {relays
                                            .into_iter()
                                            .map(|relay| view! { <li class="text-xs text-on-surface-variant py-1">{relay}</li> })
                                            .collect::<Vec<_>>()}
                                    </ul>
                                }
                                    .into_any()
                            }
                        }}
                    </div>
                </Show>

                <div class="relative hidden lg:block">
                    <input class="w-64 bg-surface-container-highest border-none rounded-md px-4 py-2 text-sm focus:ring-2 focus:ring-secondary/40 placeholder:text-on-surface-variant" placeholder={placeholder} type="text" />
                    <span class="material-symbols-outlined absolute right-3 top-2 text-on-surface-variant text-lg">"search"</span>
                </div>

                <div class="flex items-center gap-4">
                    <button class="p-2 text-[#f1f3fc]/60 hover:text-[#b6a0ff] transition-all duration-300 active:scale-95" title="Notifications">
                        <span class="material-symbols-outlined">"notifications"</span>
                    </button>
                    <button class="p-2 text-[#f1f3fc]/60 hover:text-[#b6a0ff] transition-all duration-300 active:scale-95" title="Cart">
                        <span class="material-symbols-outlined">"shopping_cart"</span>
                    </button>
                    <button class="rounded-full" on:click=move |_| on_open_profile.run(())>
                        <img
                            alt={move || display_name.get()}
                            class="w-10 h-10 rounded-full border-2 border-primary-dim/20"
                            src="https://lh3.googleusercontent.com/aida-public/AB6AXuBlBb9Z2XKeIyly4E0jzQKQL1WrIRbvYtjoErPatKPOVPljRli_-0vSEhy9ulHT1c80OBEZ9Tbw2Iuk89j1eY0ufF4rVHfnwfzruhtjb0-gduo9w0weQ330SmROvJ5UXj4LH5xobya_kUQS0C5jVapaNkz_kSUDeP6YGOVpn75RKAgLTaUuEUDLLoI2M5r2uULkULPALtpeNGk4c9lS1sPFMF_6pHMZ6393yOUr_WV1jeTn0o1bsnwCjzZxpoJ1oWBsWxZ6jnMhyJA"
                        />
                    </button>
                </div>
            </div>
        </header>
    }
}
