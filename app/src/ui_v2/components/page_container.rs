use leptos::prelude::*;

#[component]
pub fn PageContainer(
    children: Children,
    #[prop(optional)] wide: bool,
    #[prop(optional)] full_height: bool,
) -> impl IntoView {
    let class = match (wide, full_height) {
        (true, true) => "arc-page-container arc-page-container-wide arc-page-container-full",
        (true, false) => "arc-page-container arc-page-container-wide",
        (false, true) => "arc-page-container arc-page-container-full",
        (false, false) => "arc-page-container",
    };

    view! { <div class=class>{children()}</div> }
}

#[component]
pub fn ClippedPanel(
    children: Children,
    #[prop(optional, into)] class: Option<String>,
) -> impl IntoView {
    let class = class
        .map(|class| format!("arc-clipped-panel {class}"))
        .unwrap_or_else(|| "arc-clipped-panel".to_string());

    view! { <div class=class>{children()}</div> }
}
