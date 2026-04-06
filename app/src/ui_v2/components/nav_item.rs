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
            class={move || {
                if active.get() {
                    "flex items-center gap-4 bg-[#1b2028] text-[#b6a0ff] rounded-lg px-4 py-3 mx-2 my-1 cursor-pointer active:opacity-80 transition-transform duration-200 hover:translate-x-1 font-['Inter'] text-sm font-medium w-[calc(100%-1rem)] text-left"
                } else {
                    "flex items-center gap-4 text-[#f1f3fc]/50 px-4 py-3 mx-2 my-1 hover:bg-[#262c36]/30 cursor-pointer active:opacity-80 transition-transform duration-200 hover:translate-x-1 font-['Inter'] text-sm font-medium w-[calc(100%-1rem)] text-left rounded-lg"
                }
            }}
            on:click=move |_| on_click.run(())
        >
            <span class="material-symbols-outlined">
                {icon.unwrap_or("circle")}
            </span>
            <span>{label}</span>
        </button>
    }
}
