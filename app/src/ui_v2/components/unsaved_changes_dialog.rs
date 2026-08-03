//! Unsaved-navigation guard.
//!
//! The editors in this application hold author input in memory. Leaving one
//! with unsaved input must be a decision, not an accident.
//!
//! The guard is deliberately narrow. It reports only what the existing
//! dirty-state signals already know, it never claims a Save that does not
//! exist, and it never blocks navigation away from a clean editor.

use leptos::prelude::*;

use super::dialog::{
    Dialog, DialogCloseAction, DialogClosePolicy, DialogCloseRequest, DialogDismissal,
    DialogInitialFocus, DialogTone, DialogWidth,
};

thread_local! {
    /// Create Game has no draft persistence at all (`SUPPORTS_DRAFTS` is
    /// false), so its dirty state exists only while the form is mounted. The
    /// form publishes it here so the shell can guard navigation without
    /// reaching into the component.
    static CREATE_GAME_DIRTY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Publish the Create Game form's dirty state. Cleared when it unmounts.
pub fn set_create_game_dirty(dirty: bool) {
    CREATE_GAME_DIRTY.with(|flag| flag.set(dirty));
}

/// Whether a mounted Create Game form currently holds unsaved input.
pub fn create_game_dirty() -> bool {
    CREATE_GAME_DIRTY.with(std::cell::Cell::get)
}

/// An editor holding unsaved in-memory input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnsavedWork {
    /// Create Game. No draft persistence exists for this form at all.
    CreateGame,
    /// The Store Page editor, holding a dirty in-session draft.
    StorePage,
}

impl UnsavedWork {
    pub fn title(self) -> &'static str {
        match self {
            Self::CreateGame => "Discard unsaved game details?",
            Self::StorePage => "Discard unsaved Store Page changes?",
        }
    }

    /// States precisely what is lost. Neither editor writes to disk, so both
    /// messages talk about in-memory input only.
    pub fn message(self) -> &'static str {
        match self {
            Self::CreateGame => {
                "Your Create Game inputs are held in memory only and will be discarded. Arcadestr does not save Create Game drafts, and nothing has been published."
            }
            Self::StorePage => {
                "Your unsaved in-memory Store Page changes will be discarded. Anything already published to relays is unaffected."
            }
        }
    }

    pub fn keep_label(self) -> &'static str {
        "Keep editing"
    }

    pub fn discard_label(self) -> &'static str {
        "Discard and continue"
    }
}

/// What a navigation attempt should do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavigationGuard {
    /// Nothing is unsaved. Navigate immediately.
    Proceed,
    /// Ask before leaving.
    Confirm(UnsavedWork),
}

/// Decide whether a navigation attempt needs confirmation.
///
/// A clean editor is never guarded, and a screen that is not an editor has no
/// unsaved work to report in the first place.
pub fn guard_navigation(unsaved: Option<UnsavedWork>) -> NavigationGuard {
    match unsaved {
        Some(work) => NavigationGuard::Confirm(work),
        None => NavigationGuard::Proceed,
    }
}

/// How the user answered the guard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuardResolution {
    /// Stay where you are; the pending destination is dropped.
    KeepEditing,
    /// Drop the unsaved input and go to the pending destination.
    DiscardAndContinue,
}

/// Map a dialog close request onto a guard resolution.
///
/// Escape, the close control, and "Keep editing" all mean the same thing: the
/// user stays in the editor. Only the explicit discard action leaves.
pub fn resolve_guard(action: DialogCloseAction) -> Option<GuardResolution> {
    match action {
        DialogCloseAction::Dismiss => Some(GuardResolution::KeepEditing),
        _ => None,
    }
}

/// The guard dialog. Presentation only: the caller owns the pending
/// destination and performs the actual discard.
#[component]
pub fn UnsavedChangesDialog(
    /// The unsaved work behind the pending navigation, if any.
    #[prop(into)]
    work: Signal<Option<UnsavedWork>>,
    /// The user chose to stay.
    on_keep_editing: Callback<()>,
    /// The user chose to drop the unsaved input and continue.
    on_discard: Callback<()>,
) -> impl IntoView {
    let keep_ref = NodeRef::<leptos::html::Button>::new();

    view! {
        <Dialog
            id="unsaved-changes"
            open=Signal::derive(move || work.get().is_some())
            title=Signal::derive(move || work.get().map(UnsavedWork::title).unwrap_or_default().to_string())
            kicker="Unsaved changes"
            description=Signal::derive(move || work.get().map(UnsavedWork::message).unwrap_or_default().to_string())
            width=DialogWidth::Compact
            tone=DialogTone::Destructive
            policy=DialogClosePolicy::Dismissible
            dismissal=DialogDismissal::keyboard_and_button()
            // Staying in the editor is the safe outcome, so it takes focus.
            initial_focus=DialogInitialFocus::Button(keep_ref)
            close_label="Keep editing"
            on_close=UnsyncCallback::new(move |request: DialogCloseRequest| {
                if resolve_guard(request.action) == Some(GuardResolution::KeepEditing) {
                    on_keep_editing.run(());
                }
            })
            actions=move || view! {
                <button
                    node_ref=keep_ref
                    type="button"
                    class="v2-btn-secondary"
                    on:click=move |_| on_keep_editing.run(())
                >{move || work.get().map(UnsavedWork::keep_label).unwrap_or_default()}</button>
                <button
                    type="button"
                    class="v2-btn-danger"
                    on:click=move |_| on_discard.run(())
                >{move || work.get().map(UnsavedWork::discard_label).unwrap_or_default()}</button>
            }
        />
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_navigation_is_never_blocked() {
        assert_eq!(guard_navigation(None), NavigationGuard::Proceed);
    }

    #[test]
    fn dirty_navigation_is_confirmed_per_editor() {
        assert_eq!(
            guard_navigation(Some(UnsavedWork::CreateGame)),
            NavigationGuard::Confirm(UnsavedWork::CreateGame)
        );
        assert_eq!(
            guard_navigation(Some(UnsavedWork::StorePage)),
            NavigationGuard::Confirm(UnsavedWork::StorePage)
        );
    }

    #[test]
    fn dismissing_the_guard_keeps_the_user_in_the_editor() {
        assert_eq!(
            resolve_guard(DialogCloseAction::Dismiss),
            Some(GuardResolution::KeepEditing)
        );
        assert_eq!(resolve_guard(DialogCloseAction::Ignore), None);
    }

    #[test]
    fn the_guard_never_offers_a_save_that_does_not_exist() {
        for work in [UnsavedWork::CreateGame, UnsavedWork::StorePage] {
            assert_eq!(work.keep_label(), "Keep editing");
            assert_eq!(work.discard_label(), "Discard and continue");
            assert!(!work.discard_label().to_lowercase().contains("save"));
            assert!(!work.keep_label().to_lowercase().contains("save"));
        }
    }

    #[test]
    fn the_guard_states_that_in_memory_changes_are_discarded() {
        for work in [UnsavedWork::CreateGame, UnsavedWork::StorePage] {
            let message = work.message();
            assert!(message.contains("in memory") || message.contains("in-memory"));
            assert!(message.contains("discarded"));
        }
    }
}
