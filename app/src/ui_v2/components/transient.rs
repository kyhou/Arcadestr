//! Coordination for transient menus and popovers.
//!
//! Transient surfaces are disclosures, not modals: they must not trap focus and
//! must not be converted into dialogs. What they do share with dialogs is a
//! dismissal contract — Escape closes, an outside pointer press closes, focus
//! returns to the invoker, and opening a modal closes them so nothing is left
//! stranded behind the modal scrim.

use leptos::prelude::*;

thread_local! {
    static MODAL_EPOCH: std::cell::RefCell<Option<ArcRwSignal<u64>>> =
        const { std::cell::RefCell::new(None) };
}

/// Process-wide counter incremented every time a modal dialog opens.
///
/// An `ArcRwSignal` rather than an arena `RwSignal`: this outlives any single
/// component owner, so it must not be disposed with the first component that
/// happens to touch it.
fn modal_epoch() -> ArcRwSignal<u64> {
    MODAL_EPOCH.with(|slot| {
        let mut slot = slot.borrow_mut();
        slot.get_or_insert_with(|| ArcRwSignal::new(0)).clone()
    })
}

/// Announce that a modal dialog just opened. Called by the dialog primitive.
pub fn notify_modal_opened() {
    modal_epoch().update(|epoch| *epoch = epoch.wrapping_add(1));
}

/// Whether a transient overlay should close given the epoch it last observed.
pub fn should_close_for_modal(seen_epoch: u64, current_epoch: u64) -> bool {
    seen_epoch != current_epoch
}

/// Whether an Escape key press should close a transient overlay.
pub fn should_close_on_escape(key: &str, is_open: bool) -> bool {
    is_open && key == "Escape"
}

/// Whether an outside pointer press should close a transient overlay.
///
/// `inside_container` is the result of walking up from the event target to the
/// overlay's own container, so a click on the overlay's own contents (or on its
/// trigger) never dismisses it.
pub fn should_close_on_outside_pointer(is_open: bool, inside_container: bool) -> bool {
    is_open && !inside_container
}

/// Close `open` whenever a modal dialog opens.
pub fn close_transient_when_modal_opens(open: RwSignal<bool>) {
    let epoch = modal_epoch();
    let seen = StoredValue::new(epoch.get_untracked());
    Effect::new(move |_| {
        let current = epoch.get();
        if should_close_for_modal(seen.get_value(), current) {
            seen.set_value(current);
            if open.get_untracked() {
                open.set(false);
            }
        }
    });
}

/// Close `open` when a pointer press lands outside `container_selector`.
///
/// Focus is deliberately not moved: the user is already interacting with
/// whatever they pressed. Escape and the explicit close control are the paths
/// that restore focus to the invoker.
#[cfg(target_arch = "wasm32")]
pub fn close_transient_on_outside_pointer(
    open: RwSignal<bool>,
    container_selector: impl Into<String>,
) {
    use wasm_bindgen::JsCast;

    let container_selector = container_selector.into();
    let handle = window_event_listener(leptos::ev::pointerdown, move |event| {
        if !open.get_untracked() {
            return;
        }
        let inside = event
            .target()
            .and_then(|target| target.dyn_into::<web_sys::Element>().ok())
            .and_then(|element| element.closest(&container_selector).ok().flatten())
            .is_some();
        if should_close_on_outside_pointer(true, inside) {
            open.set(false);
        }
    });
    on_cleanup(move || handle.remove());
}

#[cfg(not(target_arch = "wasm32"))]
pub fn close_transient_on_outside_pointer(
    _open: RwSignal<bool>,
    _container_selector: impl Into<String>,
) {
}

/// Move focus back to the control that opened a transient overlay.
#[cfg(target_arch = "wasm32")]
pub fn focus_transient_invoker(id: &str) {
    use wasm_bindgen::JsCast;

    if let Some(element) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id(id))
        .and_then(|element| element.dyn_into::<web_sys::HtmlElement>().ok())
    {
        let _ = element.focus();
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn focus_transient_invoker(_id: &str) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_transient_overlay_closes_when_a_modal_opens() {
        assert!(should_close_for_modal(3, 4));
        assert!(!should_close_for_modal(4, 4));
    }

    #[test]
    fn escape_closes_only_an_open_transient_overlay() {
        assert!(should_close_on_escape("Escape", true));
        assert!(!should_close_on_escape("Escape", false));
        assert!(!should_close_on_escape("Enter", true));
    }

    #[test]
    fn an_outside_pointer_press_closes_but_an_inside_one_does_not() {
        assert!(should_close_on_outside_pointer(true, false));
        assert!(!should_close_on_outside_pointer(true, true));
        assert!(!should_close_on_outside_pointer(false, false));
    }
}
