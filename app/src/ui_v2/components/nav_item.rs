use leptos::prelude::*;

#[component]
pub fn NavItem(
    label: &'static str,
    #[prop(optional)] icon: Option<&'static str>,
    active: Signal<bool>,
    on_click: Callback<()>,
) -> impl IntoView {
    view! {
        <button
            type="button"
            class={move || {
                if active.get() {
                    "group flex w-full items-center gap-3 rounded-xl bg-surface-container-high px-3 py-2.5 text-left text-sm font-semibold text-on-surface outline-none ring-primary/60 transition-colors focus-visible:ring-2"
                } else {
                    "group flex w-full items-center gap-3 rounded-xl px-3 py-2.5 text-left text-sm font-medium text-on-surface-variant outline-none ring-primary/60 transition-colors hover:bg-surface-container-high/60 hover:text-on-surface focus-visible:ring-2"
                }
            }}
            aria-current={move || active.get().then_some("page")}
            on:click=move |_| on_click.run(())
        >
            <span class="material-symbols-outlined text-lg" aria-hidden="true">
                {icon.unwrap_or("circle")}
            </span>
            <span>{label}</span>
            <span
                class={move || if active.get() {
                    "ml-auto h-1.5 w-1.5 rounded-full bg-primary"
                } else {
                    "hidden"
                }}
                aria-hidden="true"
            ></span>
        </button>
    }
}

#[component]
pub fn MobileNavItem(
    label: &'static str,
    #[prop(optional)] icon: Option<&'static str>,
    active: Signal<bool>,
    on_click: Callback<()>,
) -> impl IntoView {
    view! {
        <button
            type="button"
            class={move || if active.get() {
                "flex w-full items-center gap-3 rounded-xl bg-surface-container-high px-4 py-3 text-left text-sm font-semibold text-primary outline-none ring-primary/60 focus-visible:ring-2"
            } else {
                "flex w-full items-center gap-3 rounded-xl px-4 py-3 text-left text-sm font-medium text-on-surface-variant outline-none ring-primary/60 hover:bg-surface-container-high/60 hover:text-on-surface focus-visible:ring-2"
            }}
            aria-current={move || active.get().then_some("page")}
            on:click=move |_| on_click.run(())
        >
            <span class="material-symbols-outlined text-xl" aria-hidden="true">
                {icon.unwrap_or("circle")}
            </span>
            <span>{label}</span>
            <Show when=move || active.get()>
                <span class="ml-auto text-xs font-semibold uppercase tracking-widest text-primary">"Current"</span>
            </Show>
        </button>
    }
}
