//! Background inertness for open modal dialogs.
//!
//! `HTMLDialogElement::show_modal()` already makes the rest of the document
//! *blocked by a modal dialog*: the top layer takes pointer events, focus is
//! trapped inside the dialog, and every other node is marked inert for
//! assistive technology. That covers keyboard, pointer, and accessibility-tree
//! exposure without any help from us.
//!
//! What it does not cover is **scrolling**. The document behind an open dialog
//! still scrolls under the wheel and under keyboard scroll keys routed to the
//! viewport, so the background drifts while a dialog is up. That is the one
//! real gap, and it is what this module closes.
//!
//! The `inert` attribute is deliberately *not* applied to the application
//! shell. Every dialog is rendered inside the shell subtree, and the HTML
//! inertness rules carve out no exception for a modal dialog nested under an
//! explicitly inert ancestor — setting `inert` on the shell would make the
//! dialog itself inert and leave the user with an unreachable modal.
//!
//! # Why not `overflow: hidden`
//!
//! The obvious lock — `overflow: hidden` on the root — does not work in the
//! WebKitGTK webview the desktop shell uses. Measured in the running app with
//! the Settings route scrolled (page height 2397px, viewport 752px):
//!
//! | technique                                        | background locked |
//! |--------------------------------------------------|-------------------|
//! | `html { overflow: hidden }`                       | no (scrolled 600) |
//! | `html, body { overflow: hidden }`                 | no (scrolled 600) |
//! | `html, body { height: 100%; overflow: hidden }`   | yes, but jumps    |
//! | `body { position: fixed; top: -scrollY }`         | yes               |
//!
//! The height-constrained variant locks but destroys the scroll offset: the
//! page snapped from y=500 to y=0 the moment a dialog opened, and stayed there
//! after it closed. Only the scroll-compensating fixed-body lock both holds the
//! background still and keeps it visually where the user left it, so that is
//! what this module does: record the offset, pin the body at its negation, and
//! scroll back to the recorded offset on release.
//!
//! The lock is reference counted. Dialogs stack (a confirmation raised from a
//! dialog, or a route change that mounts a new dialog before the old one
//! unmounts), and a naive boolean would unlock the page as soon as the *first*
//! dialog closed, while another was still open. Only the outermost acquire
//! records the scroll offset, so a stacked dialog cannot overwrite it with the
//! already-pinned value of 0.

/// Whether the document-level scroll lock should be applied or removed as the
/// count of open modals moves from `before` to `after`.
///
/// Separated from the DOM so the lifecycle is testable off the browser: the
/// failure mode that matters is a lock that outlives the last dialog, which
/// leaves the whole application unscrollable with nothing on screen to explain
/// it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollLockAction {
    /// First modal opened: lock the document.
    Lock,
    /// Last modal closed: release the document.
    Release,
    /// A modal opened or closed while others remain; the lock state is correct.
    Unchanged,
}

/// Pure transition for the open-modal reference count.
pub fn scroll_lock_transition(before: u32, after: u32) -> ScrollLockAction {
    match (before, after) {
        (0, a) if a > 0 => ScrollLockAction::Lock,
        (b, 0) if b > 0 => ScrollLockAction::Release,
        _ => ScrollLockAction::Unchanged,
    }
}

/// Class applied to `<body>` while any modal dialog is open. Pairs with an
/// inline `top` offset set by [`acquire`].
pub const MODAL_OPEN_CLASS: &str = "arc-modal-open";

#[cfg(target_arch = "wasm32")]
mod dom {
    use super::*;
    use std::cell::Cell;
    use wasm_bindgen::JsCast;

    thread_local! {
        static OPEN_MODALS: Cell<u32> = const { Cell::new(0) };
        /// Scroll offset captured when the outermost modal opened.
        static SAVED_SCROLL: Cell<f64> = const { Cell::new(0.0) };
    }

    fn apply(action: ScrollLockAction) {
        let Some(window) = web_sys::window() else {
            return;
        };
        let Some(body) = window
            .document()
            .and_then(|document| document.body())
            .and_then(|body| body.dyn_into::<web_sys::HtmlElement>().ok())
        else {
            return;
        };
        match action {
            ScrollLockAction::Lock => {
                let offset = window.scroll_y().unwrap_or(0.0);
                SAVED_SCROLL.with(|saved| saved.set(offset));
                // The inline offset is the whole point of the technique and
                // cannot live in the stylesheet; the class carries the rest.
                let _ = body.style().set_property("top", &format!("{}px", -offset));
                let _ = body.class_list().add_1(MODAL_OPEN_CLASS);
            }
            ScrollLockAction::Release => {
                let _ = body.class_list().remove_1(MODAL_OPEN_CLASS);
                let _ = body.style().remove_property("top");
                let offset = SAVED_SCROLL.with(|saved| saved.replace(0.0));
                window.scroll_to_with_x_and_y(0.0, offset);
            }
            ScrollLockAction::Unchanged => {}
        }
    }

    /// Register one open modal. Idempotent per holder: callers must pair this
    /// with exactly one [`release`].
    pub fn acquire() {
        OPEN_MODALS.with(|count| {
            let before = count.get();
            let after = before.saturating_add(1);
            count.set(after);
            apply(scroll_lock_transition(before, after));
        });
    }

    /// Release one open modal.
    pub fn release() {
        OPEN_MODALS.with(|count| {
            let before = count.get();
            let after = before.saturating_sub(1);
            count.set(after);
            apply(scroll_lock_transition(before, after));
        });
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod dom {
    pub fn acquire() {}
    pub fn release() {}
}

pub use dom::{acquire, release};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_modal_locks_and_the_last_one_releases() {
        assert_eq!(scroll_lock_transition(0, 1), ScrollLockAction::Lock);
        assert_eq!(scroll_lock_transition(1, 0), ScrollLockAction::Release);
    }

    #[test]
    fn a_stacked_modal_neither_relocks_nor_releases_early() {
        // A confirmation raised from inside another dialog must not release the
        // lock when it closes while its parent dialog is still open.
        assert_eq!(scroll_lock_transition(1, 2), ScrollLockAction::Unchanged);
        assert_eq!(scroll_lock_transition(2, 1), ScrollLockAction::Unchanged);
    }

    #[test]
    fn only_the_outermost_transition_touches_the_saved_scroll_offset() {
        // The saved offset is read on Lock and restored on Release. If a
        // stacked dialog reported Lock, it would capture the pinned offset of
        // 0 and the page would jump to the top when everything closed.
        assert_eq!(scroll_lock_transition(1, 2), ScrollLockAction::Unchanged);
        assert_eq!(scroll_lock_transition(2, 3), ScrollLockAction::Unchanged);
        assert_eq!(scroll_lock_transition(3, 2), ScrollLockAction::Unchanged);
        assert_eq!(scroll_lock_transition(1, 0), ScrollLockAction::Release);
    }

    #[test]
    fn an_unbalanced_release_cannot_wedge_the_page_scrolled_shut() {
        // Saturating arithmetic means an extra release lands on 0 rather than
        // wrapping to u32::MAX, which would leave the document locked forever.
        assert_eq!(scroll_lock_transition(0, 0), ScrollLockAction::Unchanged);
    }
}
