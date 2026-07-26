use leptos::ev::MouseEvent;
use leptos::prelude::*;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ButtonVariant {
    Primary,
    #[default]
    Secondary,
    Ghost,
    Danger,
}

impl ButtonVariant {
    fn class(self) -> &'static str {
        match self {
            Self::Primary => "v2-btn-primary",
            Self::Secondary => "v2-btn-secondary",
            Self::Ghost => "v2-btn-ghost",
            Self::Danger => "v2-btn-danger",
        }
    }
}

#[component]
pub fn Button(
    #[prop(into)] on_click: Callback<MouseEvent>,
    children: Children,
    #[prop(optional)] variant: ButtonVariant,
    #[prop(optional, into)] disabled: MaybeProp<bool>,
    #[prop(optional, into)] class: Option<String>,
) -> impl IntoView {
    let class = match class {
        Some(class) => format!("{} {class}", variant.class()),
        None => variant.class().to_string(),
    };

    view! {
        <button
            type="button"
            class=class
            disabled=move || disabled.get().unwrap_or(false)
            on:click=move |event| on_click.run(event)
        >
            {children()}
        </button>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variants_map_to_shared_theme_classes() {
        assert_eq!(ButtonVariant::Primary.class(), "v2-btn-primary");
        assert_eq!(ButtonVariant::Secondary.class(), "v2-btn-secondary");
        assert_eq!(ButtonVariant::Ghost.class(), "v2-btn-ghost");
        assert_eq!(ButtonVariant::Danger.class(), "v2-btn-danger");
    }
}
