use leptos::ev::MouseEvent;
use leptos::prelude::*;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ButtonVariant {
    Primary,
    Action,
    #[default]
    Secondary,
    Neutral,
    Ghost,
    Danger,
    Success,
}

impl ButtonVariant {
    fn class(self) -> &'static str {
        match self {
            Self::Primary => "v2-btn-primary",
            Self::Action => "v2-btn-action",
            Self::Secondary => "v2-btn-secondary",
            Self::Neutral => "v2-btn-neutral",
            Self::Ghost => "v2-btn-ghost",
            Self::Danger => "v2-btn-danger",
            Self::Success => "v2-btn-success",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ButtonSize {
    Compact,
    #[default]
    Standard,
    Large,
}

impl ButtonSize {
    fn class(self) -> &'static str {
        match self {
            Self::Compact => "arc-btn-compact",
            Self::Standard => "arc-btn-standard",
            Self::Large => "arc-btn-large",
        }
    }
}

const fn can_activate(disabled: bool, busy: bool) -> bool {
    !disabled && !busy
}

#[component]
pub fn Button(
    #[prop(into)] on_click: Callback<MouseEvent>,
    children: Children,
    #[prop(optional)] variant: ButtonVariant,
    #[prop(optional)] size: ButtonSize,
    #[prop(optional, into)] disabled: MaybeProp<bool>,
    #[prop(optional, into)] busy: MaybeProp<bool>,
    #[prop(optional, into)] busy_label: Option<String>,
    #[prop(optional)] clipped: bool,
    #[prop(optional, into)] class: Option<String>,
) -> impl IntoView {
    let class = format!(
        "{} {} {} {}",
        variant.class(),
        size.class(),
        clipped.then_some("arc-btn-clipped").unwrap_or_default(),
        class.unwrap_or_default()
    );
    let busy_text = busy_label.unwrap_or_else(|| "Working".to_string());

    view! {
        <button
            type="button"
            class=class
            disabled=move || disabled.get().unwrap_or(false) || busy.get().unwrap_or(false)
            aria-busy=move || busy.get().unwrap_or(false).then_some("true")
            on:click=move |event| {
                if can_activate(
                    disabled.get_untracked().unwrap_or(false),
                    busy.get_untracked().unwrap_or(false),
                ) {
                    on_click.run(event);
                }
            }
        >
            <span class="arc-btn-content" class:arc-btn-content-hidden=move || busy.get().unwrap_or(false)>
                {children()}
            </span>
            <Show when=move || busy.get().unwrap_or(false)>
                <span class="arc-btn-busy" role="status">
                    <span class="arc-loading-mark" aria-hidden="true"></span>
                    <span>{busy_text.clone()}</span>
                </span>
            </Show>
        </button>
    }
}

#[component]
pub fn IconButton(
    icon: &'static str,
    #[prop(into)] aria_label: String,
    #[prop(into)] on_click: Callback<MouseEvent>,
    #[prop(optional, into)] disabled: MaybeProp<bool>,
    #[prop(optional, into)] busy: MaybeProp<bool>,
    #[prop(optional, into)] class: Option<String>,
) -> impl IntoView {
    let class = class
        .map(|class| format!("arc-icon-button {class}"))
        .unwrap_or_else(|| "arc-icon-button".to_string());

    view! {
        <button
            type="button"
            class=class
            aria-label=aria_label
            aria-busy=move || busy.get().unwrap_or(false).then_some("true")
            disabled=move || disabled.get().unwrap_or(false) || busy.get().unwrap_or(false)
            on:click=move |event| {
                if can_activate(
                    disabled.get_untracked().unwrap_or(false),
                    busy.get_untracked().unwrap_or(false),
                ) {
                    on_click.run(event);
                }
            }
        >
            <Show
                when=move || !busy.get().unwrap_or(false)
                fallback=|| view! { <span class="arc-loading-mark" aria-hidden="true"></span> }
            >
                <span class="material-symbols-outlined" aria-hidden="true">{icon}</span>
            </Show>
        </button>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variants_map_to_shared_theme_classes() {
        assert_eq!(ButtonVariant::Primary.class(), "v2-btn-primary");
        assert_eq!(ButtonVariant::Action.class(), "v2-btn-action");
        assert_eq!(ButtonVariant::Secondary.class(), "v2-btn-secondary");
        assert_eq!(ButtonVariant::Neutral.class(), "v2-btn-neutral");
        assert_eq!(ButtonVariant::Ghost.class(), "v2-btn-ghost");
        assert_eq!(ButtonVariant::Danger.class(), "v2-btn-danger");
        assert_eq!(ButtonVariant::Success.class(), "v2-btn-success");
    }

    #[test]
    fn disabled_or_busy_buttons_cannot_activate() {
        assert!(can_activate(false, false));
        assert!(!can_activate(true, false));
        assert!(!can_activate(false, true));
        assert!(!can_activate(true, true));
    }
}
