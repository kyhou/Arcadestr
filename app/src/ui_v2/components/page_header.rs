use leptos::prelude::*;

#[component]
pub fn PageHeader(
    title: String,
    #[prop(optional)] eyebrow: Option<String>,
    #[prop(optional)] description: Option<String>,
    #[prop(optional)] action: Option<AnyView>,
) -> impl IntoView {
    view! {
        <header class="mb-8 flex flex-wrap items-end justify-between gap-4">
            <div class="min-w-0">
                {eyebrow.map(|eyebrow| view! {
                    <p class="mb-2 text-xs font-semibold uppercase tracking-[0.2em] text-secondary">
                        {eyebrow}
                    </p>
                })}
                <h1 class="font-display text-4xl font-bold leading-tight text-on-surface md:text-5xl">
                    {title}
                </h1>
                {description.map(|description| view! {
                    <p class="mt-2 max-w-2xl text-sm leading-relaxed text-on-surface-variant md:text-base">
                        {description}
                    </p>
                })}
            </div>
            {action.map(|action| view! { <div class="shrink-0">{action}</div> })}
        </header>
    }
}
