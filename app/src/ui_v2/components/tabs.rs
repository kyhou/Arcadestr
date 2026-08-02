use std::sync::atomic::{AtomicUsize, Ordering};

use leptos::ev::KeyboardEvent;
use leptos::prelude::*;

static TAB_GROUP_ID: AtomicUsize = AtomicUsize::new(1);

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PageTabSemantics {
    #[default]
    Tabs,
    Filter,
    Navigation,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PageTabTarget {
    Local,
    Route(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PageTabItem {
    pub id: String,
    pub label: String,
    pub disabled: bool,
    pub disabled_reason: Option<String>,
    pub target: PageTabTarget,
}

impl PageTabItem {
    pub fn local(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            disabled: false,
            disabled_reason: None,
            target: PageTabTarget::Local,
        }
    }

    pub fn route(id: impl Into<String>, label: impl Into<String>, href: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            disabled: false,
            disabled_reason: None,
            target: PageTabTarget::Route(href.into()),
        }
    }

    pub fn disabled(mut self) -> Self {
        self.disabled = true;
        self.disabled_reason = Some("Unavailable".to_string());
        self
    }

    pub fn disabled_with_reason(mut self, reason: impl Into<String>) -> Self {
        self.disabled = true;
        self.disabled_reason = Some(reason.into());
        self
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TabMove {
    Previous,
    Next,
    First,
    Last,
}

fn tab_move_for_key(key: &str) -> Option<TabMove> {
    match key {
        "ArrowLeft" => Some(TabMove::Previous),
        "ArrowRight" => Some(TabMove::Next),
        "Home" => Some(TabMove::First),
        "End" => Some(TabMove::Last),
        _ => None,
    }
}

fn next_enabled_index(items: &[PageTabItem], current: usize, movement: TabMove) -> Option<usize> {
    if items.is_empty() || items.iter().all(|item| item.disabled) {
        return None;
    }

    match movement {
        TabMove::First => items.iter().position(|item| !item.disabled),
        TabMove::Last => items.iter().rposition(|item| !item.disabled),
        TabMove::Previous | TabMove::Next => {
            let step = if movement == TabMove::Next {
                1
            } else {
                items.len() - 1
            };
            (1..=items.len())
                .map(|offset| (current + offset * step) % items.len())
                .find(|index| !items[*index].disabled)
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn focus_tab(id: &str) {
    use wasm_bindgen::JsCast;

    if let Some(element) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id(id))
        .and_then(|element| element.dyn_into::<web_sys::HtmlElement>().ok())
    {
        let _ = element.focus();
    }
}

#[cfg(target_arch = "wasm32")]
fn activate_route_tab(id: &str) {
    use wasm_bindgen::JsCast;

    if let Some(element) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id(id))
        .and_then(|element| element.dyn_into::<web_sys::HtmlElement>().ok())
    {
        let _ = element.focus();
        element.click();
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn focus_tab(_id: &str) {}

#[cfg(not(target_arch = "wasm32"))]
fn activate_route_tab(_id: &str) {}

#[component]
pub fn PageTabs(
    items: Vec<PageTabItem>,
    selected: Signal<String>,
    on_select: Callback<String>,
    #[prop(into)] aria_label: String,
    #[prop(optional)] semantics: PageTabSemantics,
) -> impl IntoView {
    let group_id = TAB_GROUP_ID.fetch_add(1, Ordering::Relaxed);
    let items_for_view = items.clone();

    let container_role = match semantics {
        PageTabSemantics::Tabs => Some("tablist"),
        PageTabSemantics::Filter => Some("group"),
        PageTabSemantics::Navigation => None,
    };

    view! {
        <nav
            class="arc-page-tabs"
            class:arc-page-tabs-filter=semantics == PageTabSemantics::Filter
            aria-label=aria_label
            role=container_role
        >
            {items_for_view
                .into_iter()
                .enumerate()
                .map(|(index, item)| {
                    let item_id = format!("arc-page-tab-{group_id}-{index}");
                    let item_id_for_key = item_id.clone();
                    let item_key = item.id.clone();
                    let item_key_for_click = item_key.clone();
                    let items_for_key = items.clone();
                    let keyboard_navigation = semantics != PageTabSemantics::Navigation;
                    let active = Signal::derive(move || selected.get() == item_key);
                    let on_keydown = move |event: KeyboardEvent| {
                        if !keyboard_navigation {
                            return;
                        }
                        let Some(movement) = tab_move_for_key(&event.key()) else {
                            return;
                        };
                        let Some(next) = next_enabled_index(&items_for_key, index, movement) else {
                            return;
                        };
                        event.prevent_default();
                        let next_item = &items_for_key[next];
                        let next_id = format!("arc-page-tab-{group_id}-{next}");
                        if matches!(next_item.target, PageTabTarget::Route(_)) {
                            activate_route_tab(&next_id);
                        } else {
                            on_select.run(next_item.id.clone());
                            focus_tab(&next_id);
                        }
                    };

                    match item.target {
                        PageTabTarget::Local => view! {
                            <button
                                id=item_id_for_key
                                type="button"
                                class="arc-page-tab"
                                class:arc-page-tab-active=move || active.get()
                                role=(semantics == PageTabSemantics::Tabs).then_some("tab")
                                aria-selected=move || (semantics == PageTabSemantics::Tabs).then(|| active.get().to_string())
                                aria-pressed=move || (semantics == PageTabSemantics::Filter).then(|| active.get().to_string())
                                aria-current=move || (semantics == PageTabSemantics::Navigation && active.get()).then_some("page")
                                aria-disabled=item.disabled.then_some("true")
                                title=item.disabled_reason.clone()
                                tabindex=move || {
                                    if (item.disabled && semantics != PageTabSemantics::Navigation)
                                        || (semantics == PageTabSemantics::Tabs && !active.get()) {
                                        "-1"
                                    } else {
                                        "0"
                                    }
                                }
                                disabled=item.disabled && semantics != PageTabSemantics::Navigation
                                on:click=move |_| {
                                    if !item.disabled {
                                        on_select.run(item_key_for_click.clone());
                                    }
                                }
                                on:keydown=on_keydown
                            >
                                {item.label}
                                {item.disabled.then(|| view! {
                                    <span class="material-symbols-outlined arc-tab-disabled-icon" aria-hidden="true">"block"</span>
                                })}
                                {item.disabled_reason.clone().map(|reason| view! {
                                    <span class="arc-tab-unavailable"><span aria-hidden="true">"Unavailable"</span><span class="sr-only">{reason}</span></span>
                                })}
                            </button>
                        }
                        .into_any(),
                        PageTabTarget::Route(href) => view! {
                            <a
                                id=item_id
                                href=href
                                class="arc-page-tab"
                                class:arc-page-tab-active=move || active.get()
                                role=(semantics == PageTabSemantics::Tabs).then_some("tab")
                                aria-selected=move || (semantics == PageTabSemantics::Tabs).then(|| active.get().to_string())
                                aria-current=move || active.get().then_some("page")
                                aria-disabled=item.disabled.then_some("true")
                                title=item.disabled_reason.clone()
                                tabindex=move || {
                                    if (item.disabled && semantics != PageTabSemantics::Navigation)
                                        || (semantics == PageTabSemantics::Tabs && !active.get()) {
                                        "-1"
                                    } else {
                                        "0"
                                    }
                                }
                                on:click=move |event| {
                                    if item.disabled {
                                        event.prevent_default();
                                    } else {
                                        on_select.run(item_key_for_click.clone());
                                    }
                                }
                                on:keydown=on_keydown
                            >
                                {item.label}
                                {item.disabled.then(|| view! {
                                    <span class="material-symbols-outlined arc-tab-disabled-icon" aria-hidden="true">"block"</span>
                                })}
                                {item.disabled_reason.clone().map(|reason| view! {
                                    <span class="arc-tab-unavailable"><span aria-hidden="true">"Unavailable"</span><span class="sr-only">{reason}</span></span>
                                })}
                            </a>
                        }
                        .into_any(),
                    }
                })
                .collect_view()}
        </nav>
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PublisherDestination {
    Dashboard,
    CreateGame,
    ManageGame,
    StorePage,
    Releases,
    Promotions,
    Activity,
}

impl PublisherDestination {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Dashboard => "Dashboard",
            Self::CreateGame => "Create Game",
            Self::ManageGame => "Manage Game",
            Self::StorePage => "Store Page",
            Self::Releases => "Releases",
            Self::Promotions => "Promotions",
            Self::Activity => "Activity",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PublisherTabItem {
    pub destination: PublisherDestination,
    pub enabled: bool,
    pub unavailable_reason: Option<&'static str>,
}

#[component]
pub fn PublisherTabs(
    items: Vec<PublisherTabItem>,
    active: Signal<PublisherDestination>,
    on_select: Callback<PublisherDestination>,
    #[prop(optional, into)] aria_label: Option<String>,
) -> impl IntoView {
    let page_items = items
        .iter()
        .map(|item| {
            let id = format!("{:?}", item.destination);
            let mut tab = PageTabItem::local(id, item.destination.label());
            if !item.enabled {
                tab = tab.disabled_with_reason(
                    item.unavailable_reason
                        .unwrap_or("This destination is unavailable."),
                );
            }
            tab
        })
        .collect::<Vec<_>>();
    let selected = Signal::derive(move || format!("{:?}", active.get()));
    let destination_items = items.clone();
    let activate = Callback::new(move |id: String| {
        if let Some(item) = destination_items
            .iter()
            .find(|item| format!("{:?}", item.destination) == id && item.enabled)
        {
            on_select.run(item.destination);
        }
    });

    view! {
        <div class="arc-publisher-tabs">
            <PageTabs
                items=page_items
                selected=selected
                on_select=activate
                aria_label=aria_label.unwrap_or_else(|| "Publisher navigation".to_string())
                semantics=PageTabSemantics::Navigation
            />
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyboard_navigation_skips_disabled_tabs_and_wraps() {
        let items = vec![
            PageTabItem::local("one", "One"),
            PageTabItem::local("two", "Two").disabled(),
            PageTabItem::local("three", "Three"),
        ];

        assert_eq!(next_enabled_index(&items, 0, TabMove::Next), Some(2));
        assert_eq!(next_enabled_index(&items, 2, TabMove::Next), Some(0));
        assert_eq!(next_enabled_index(&items, 0, TabMove::Previous), Some(2));
        assert_eq!(next_enabled_index(&items, 2, TabMove::First), Some(0));
        assert_eq!(next_enabled_index(&items, 0, TabMove::Last), Some(2));
    }

    #[test]
    fn tab_key_mapping_ignores_unrelated_keys() {
        assert_eq!(tab_move_for_key("ArrowRight"), Some(TabMove::Next));
        assert_eq!(tab_move_for_key("ArrowLeft"), Some(TabMove::Previous));
        assert_eq!(tab_move_for_key("Enter"), None);
    }

    #[test]
    fn publisher_destinations_have_stable_truthful_labels() {
        assert_eq!(PublisherDestination::Dashboard.label(), "Dashboard");
        assert_eq!(PublisherDestination::StorePage.label(), "Store Page");
        assert_eq!(PublisherDestination::Promotions.label(), "Promotions");
        assert_eq!(PublisherDestination::Activity.label(), "Activity");
    }
}
