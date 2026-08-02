use leptos::prelude::*;

#[component]
pub fn ArcadestrLogo(on_click: Callback<()>) -> impl IntoView {
    view! {
        <button
            type="button"
            class="arc-logo"
            aria-label="Go to Store"
            on:click=move |_| on_click.run(())
        >
            <svg
                class="arc-logo-mark"
                viewBox="0 0 20 20"
                aria-hidden="true"
                focusable="false"
            >
                <polygon points="0,6 14,0 20,6 20,20 6,20 0,14"></polygon>
            </svg>
            <span class="arc-logo-wordmark">"Arcadestr"</span>
        </button>
    }
}
