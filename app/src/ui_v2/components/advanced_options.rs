use leptos::prelude::*;

/// Native disclosure keeps optional tools keyboard-accessible without making
/// them prerequisites for the everyday flow. Closing it preserves input state.
#[component]
pub fn AdvancedOptions(
    #[prop(default = "ADVANCED OPTIONS")] title: &'static str,
    #[prop(default = "")] description: &'static str,
    children: Children,
) -> impl IntoView {
    view! {
        <details style="grid-column:1 / -1;min-width:0;width:100%;margin:12px 0;border:1px solid oklch(0.32 0.01 60);border-radius:6px;background:#121215;color:#fff;font-family:ui-monospace,monospace">
            <summary style="padding:14px 16px;cursor:pointer;font:800 13px/1.6 ui-monospace,monospace;color:oklch(0.8 0.14 195);overflow-wrap:anywhere">{title}</summary>
            <div style="padding:0 16px 16px;min-width:0;overflow-wrap:anywhere">
                {(!description.is_empty()).then(|| view! {
                    <p style="margin:0 0 16px;font:400 13px/1.7 ui-monospace,monospace;color:oklch(0.68 0.01 60)">{description}</p>
                })}
                {children()}
            </div>
        </details>
    }
}
