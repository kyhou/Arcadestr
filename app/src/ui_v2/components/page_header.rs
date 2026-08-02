use leptos::prelude::*;

#[component]
pub fn PageHeader(
    title: String,
    #[prop(optional)] eyebrow: Option<String>,
    #[prop(optional)] description: Option<String>,
    #[prop(optional)] action: Option<AnyView>,
) -> impl IntoView {
    view! {
        <header class="arc-page-header">
            <div class="arc-page-heading">
                {eyebrow.map(|eyebrow| view! {
                    <p class="arc-page-eyebrow">{eyebrow}</p>
                })}
                <h1>{title}</h1>
                {description.map(|description| view! {
                    <p class="arc-page-description">{description}</p>
                })}
            </div>
            {action.map(|action| view! { <div class="arc-page-actions">{action}</div> })}
        </header>
    }
}
