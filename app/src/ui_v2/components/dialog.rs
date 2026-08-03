//! Canonical modal dialog primitive.
//!
//! Every production dialog in `ui_v2` renders through [`Dialog`] so that
//! geometry, Escape handling, backdrop handling, close-button policy, initial
//! focus, and focus restoration are decided in exactly one place.
//!
//! The primitive owns presentation and dismissal *policy* only. It never runs
//! a backend command, never cancels an operation, and never infers behaviour
//! from button text or visual variant: the owning feature declares a typed
//! [`DialogClosePolicy`] plus a typed [`DialogDismissal`], receives a
//! [`DialogCloseRequest`], and keeps full responsibility for the existing
//! operation semantics.

use leptos::prelude::*;

/// What dismissing this dialog is allowed to do.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DialogClosePolicy {
    /// Closing has no consequence beyond hiding the dialog.
    #[default]
    Dismissible,
    /// Closing is refused while the owning operation is busy.
    BlockedWhileBusy,
    /// Closing while busy must first ask the user to confirm.
    ConfirmRequired,
    /// Closing is never offered; the owner closes the dialog programmatically.
    NonDismissible,
}

/// Whether a single dismissal channel is wired at all.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DialogSourcePolicy {
    #[default]
    Allowed,
    Ignored,
}

impl DialogSourcePolicy {
    fn is_allowed(self) -> bool {
        matches!(self, Self::Allowed)
    }
}

/// Whether the header close control exists, and whether it locks while busy.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DialogCloseButtonPolicy {
    /// No header close control is rendered.
    #[default]
    Hidden,
    /// Rendered and always actionable.
    Visible,
    /// Rendered, but disabled while the owner reports a busy operation.
    DisabledWhileBusy,
}

impl DialogCloseButtonPolicy {
    fn is_visible(self) -> bool {
        !matches!(self, Self::Hidden)
    }

    fn locks_while_busy(self) -> bool {
        matches!(self, Self::DisabledWhileBusy)
    }
}

/// Per-channel dismissal wiring, declared explicitly by the owning feature.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DialogDismissal {
    pub escape: DialogSourcePolicy,
    pub backdrop: DialogSourcePolicy,
    pub close_button: DialogCloseButtonPolicy,
}

impl Default for DialogDismissal {
    fn default() -> Self {
        Self {
            escape: DialogSourcePolicy::Allowed,
            backdrop: DialogSourcePolicy::Ignored,
            close_button: DialogCloseButtonPolicy::Hidden,
        }
    }
}

impl DialogDismissal {
    /// Escape and the close button dismiss; the backdrop is inert.
    pub fn keyboard_and_button() -> Self {
        Self {
            escape: DialogSourcePolicy::Allowed,
            backdrop: DialogSourcePolicy::Ignored,
            close_button: DialogCloseButtonPolicy::Visible,
        }
    }

    /// Every channel dismisses. Only for dialogs with no pending consequence.
    pub fn freely_dismissible() -> Self {
        Self {
            escape: DialogSourcePolicy::Allowed,
            backdrop: DialogSourcePolicy::Allowed,
            close_button: DialogCloseButtonPolicy::Visible,
        }
    }

    /// Nothing dismisses; the owner drives `open` directly.
    pub fn blocked() -> Self {
        Self {
            escape: DialogSourcePolicy::Ignored,
            backdrop: DialogSourcePolicy::Ignored,
            close_button: DialogCloseButtonPolicy::Hidden,
        }
    }

    /// Escape and the close button ask, but lock while the operation is busy.
    pub fn busy_aware() -> Self {
        Self {
            escape: DialogSourcePolicy::Allowed,
            backdrop: DialogSourcePolicy::Ignored,
            close_button: DialogCloseButtonPolicy::DisabledWhileBusy,
        }
    }
}

/// Where a close attempt came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialogCloseSource {
    /// The Escape key, delivered through the native `cancel` event.
    Escape,
    /// A click on the scrim outside the panel.
    Backdrop,
    /// The header close control.
    CloseButton,
    /// An explicit cancel/back action inside the dialog body or action row.
    Cancel,
    /// The user answered an in-dialog "abandon the operation?" prompt.
    ConfirmedCancel,
    /// The owning operation finished and the dialog is closing as a result.
    Completion,
}

/// What the owner should do about a close attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialogCloseAction {
    /// Close the dialog.
    Dismiss,
    /// Do nothing; the attempt is refused.
    Ignore,
    /// Show the owner's own confirmation before abandoning the operation.
    RequestConfirmation,
    /// Run the owner's existing cancellation flow, then close.
    CancelOperation,
}

/// A resolved close attempt handed back to the owning feature.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DialogCloseRequest {
    pub source: DialogCloseSource,
    pub action: DialogCloseAction,
}

impl DialogCloseRequest {
    pub fn dismisses(&self) -> bool {
        matches!(self.action, DialogCloseAction::Dismiss)
    }
}

/// Resolve a close attempt from policy, wiring, and busy state alone.
///
/// This is deliberately pure: no DOM, no signals, no commands.
pub fn resolve_close(
    policy: DialogClosePolicy,
    dismissal: DialogDismissal,
    busy: bool,
    source: DialogCloseSource,
) -> DialogCloseAction {
    match source {
        // The owner already finished; closing is never refused.
        DialogCloseSource::Completion => return DialogCloseAction::Dismiss,
        // The user answered the owner's confirmation prompt.
        DialogCloseSource::ConfirmedCancel => return DialogCloseAction::CancelOperation,
        DialogCloseSource::Escape if !dismissal.escape.is_allowed() => {
            return DialogCloseAction::Ignore
        }
        DialogCloseSource::Backdrop if !dismissal.backdrop.is_allowed() => {
            return DialogCloseAction::Ignore
        }
        DialogCloseSource::CloseButton if !dismissal.close_button.is_visible() => {
            return DialogCloseAction::Ignore
        }
        _ => {}
    }

    match policy {
        DialogClosePolicy::Dismissible => DialogCloseAction::Dismiss,
        DialogClosePolicy::BlockedWhileBusy => {
            if busy {
                DialogCloseAction::Ignore
            } else {
                DialogCloseAction::Dismiss
            }
        }
        DialogClosePolicy::ConfirmRequired => {
            if busy {
                DialogCloseAction::RequestConfirmation
            } else {
                DialogCloseAction::Dismiss
            }
        }
        DialogClosePolicy::NonDismissible => DialogCloseAction::Ignore,
    }
}

/// Panel width. Geometry lives in `theme.rs`, not in feature code.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DialogWidth {
    /// Confirmations and short informational dialogs.
    Compact,
    /// Forms, upload surfaces, and progress surfaces.
    #[default]
    Standard,
    /// Media expansion; fills the safe viewport area.
    Wide,
}

impl DialogWidth {
    fn class(self) -> &'static str {
        match self {
            Self::Compact => "arc-dialog-compact",
            Self::Standard => "arc-dialog-standard",
            Self::Wide => "arc-dialog-wide",
        }
    }
}

/// Presentation tone. Destructive tone is a *visual* marker only; it never
/// changes dismissal behaviour.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DialogTone {
    #[default]
    Neutral,
    Destructive,
}

impl DialogTone {
    fn is_destructive(self) -> bool {
        matches!(self, Self::Destructive)
    }
}

/// Which control should receive focus when the dialog opens.
///
/// The `Kind` form carries no DOM handle so that the selection rule stays
/// testable off the browser; [`DialogInitialFocus`] adds the actual refs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DialogInitialFocusKind {
    /// The panel itself (`tabindex="-1"`). Always safe.
    #[default]
    Panel,
    /// The header close control.
    CloseButton,
    /// A caller-provided control.
    Requested,
}

/// Where focus actually lands once availability is known.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialogFocusTarget {
    Requested,
    CloseButton,
    Panel,
}

/// Choose the initial focus target.
///
/// A requested control that is missing or disabled never receives focus; the
/// panel is the fallback so the dialog is still reachable by keyboard.
pub fn initial_focus_target(
    kind: DialogInitialFocusKind,
    requested_present: bool,
    requested_disabled: bool,
) -> DialogFocusTarget {
    match kind {
        DialogInitialFocusKind::Panel => DialogFocusTarget::Panel,
        DialogInitialFocusKind::CloseButton => {
            if requested_present && !requested_disabled {
                DialogFocusTarget::CloseButton
            } else {
                DialogFocusTarget::Panel
            }
        }
        DialogInitialFocusKind::Requested => {
            if requested_present && !requested_disabled {
                DialogFocusTarget::Requested
            } else {
                DialogFocusTarget::Panel
            }
        }
    }
}

/// Where focus returns after the dialog closes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialogFocusRestoration {
    /// The control that opened the dialog, still in the document.
    Invoker,
    /// A route-level landmark, used when the invoker is gone.
    RouteFallback,
}

/// Native `<dialog>` restores focus to the invoker on close. When the invoker
/// was unmounted while the dialog was open there is nothing to restore to, so
/// focus moves to the route landmark instead of the document body.
pub fn focus_restoration_target(invoker_connected: bool) -> DialogFocusRestoration {
    if invoker_connected {
        DialogFocusRestoration::Invoker
    } else {
        DialogFocusRestoration::RouteFallback
    }
}

/// `id` of the route-level landmark used as the focus-restoration fallback.
pub const ROUTE_FOCUS_FALLBACK_ID: &str = "arc-shell-main";

/// A caller-provided initial-focus reference.
#[derive(Clone, Copy, Default)]
pub enum DialogInitialFocus {
    /// Focus the panel. Always safe, never focuses a disabled control.
    #[default]
    Panel,
    /// Focus the header close control.
    CloseButton,
    Button(NodeRef<leptos::html::Button>),
    Input(NodeRef<leptos::html::Input>),
    TextArea(NodeRef<leptos::html::Textarea>),
}

impl DialogInitialFocus {
    fn kind(self) -> DialogInitialFocusKind {
        match self {
            Self::Panel => DialogInitialFocusKind::Panel,
            Self::CloseButton => DialogInitialFocusKind::CloseButton,
            Self::Button(_) | Self::Input(_) | Self::TextArea(_) => {
                DialogInitialFocusKind::Requested
            }
        }
    }

    fn requested_element(self) -> Option<web_sys::HtmlElement> {
        use wasm_bindgen::JsCast;
        match self {
            Self::Panel | Self::CloseButton => None,
            Self::Button(node) => node.get_untracked().map(|node| node.unchecked_into()),
            Self::Input(node) => node.get_untracked().map(|node| node.unchecked_into()),
            Self::TextArea(node) => node.get_untracked().map(|node| node.unchecked_into()),
        }
    }
}

fn element_is_disabled(element: &web_sys::HtmlElement) -> bool {
    element.has_attribute("disabled")
        || element.get_attribute("aria-disabled").as_deref() == Some("true")
}

fn active_element() -> Option<web_sys::HtmlElement> {
    use wasm_bindgen::JsCast;
    document()
        .active_element()
        .and_then(|element| element.dyn_into::<web_sys::HtmlElement>().ok())
}

fn route_fallback_element() -> Option<web_sys::HtmlElement> {
    use wasm_bindgen::JsCast;
    document()
        .get_element_by_id(ROUTE_FOCUS_FALLBACK_ID)
        .and_then(|element| element.dyn_into::<web_sys::HtmlElement>().ok())
}

/// The canonical modal dialog.
///
/// The `open` signal is authoritative: the primitive mirrors it onto the
/// native `<dialog>` but never flips it. Every dismissal attempt is reported
/// through `on_close` with a resolved [`DialogCloseAction`]; the owner decides
/// what that means for its operation and then updates `open` itself.
#[component]
#[allow(clippy::too_many_arguments)]
pub fn Dialog(
    /// Authoritative open state, owned by the calling feature.
    #[prop(into)]
    open: Signal<bool>,
    /// Stable DOM id prefix; used to build the label and description ids.
    #[prop(into)]
    id: String,
    #[prop(into)] title: Signal<String>,
    /// Small overline above the title.
    #[prop(optional, into)]
    kicker: Option<Signal<String>>,
    /// Rendered as the `aria-describedby` target when present.
    #[prop(optional, into)]
    description: Option<Signal<String>>,
    #[prop(optional)] width: DialogWidth,
    /// Presentation tone. Reactive so a dialog whose payload changes can move
    /// between neutral and destructive without changing dismissal policy.
    #[prop(optional, into)]
    tone: Option<Signal<DialogTone>>,
    /// Owner-reported busy state for the operation behind this dialog.
    #[prop(optional, into)]
    busy: Option<Signal<bool>>,
    #[prop(optional)] policy: DialogClosePolicy,
    #[prop(optional)] dismissal: DialogDismissal,
    #[prop(optional)] initial_focus: DialogInitialFocus,
    /// Announce title changes politely. Used by dialogs whose title tracks a
    /// running operation's phase.
    #[prop(optional)]
    title_live: bool,
    /// Accessible name for the header close control.
    #[prop(optional, into)]
    close_label: Option<String>,
    /// Explanation shown when the close control is locked.
    #[prop(optional, into)]
    close_blocked_hint: Option<Signal<String>>,
    /// Resolved close attempts. The owner applies the action.
    ///
    /// Unsync so owners holding non-`Send` operation handles (the Blossom
    /// upload callbacks, for one) can route their existing flows through it.
    #[prop(into)]
    on_close: UnsyncCallback<DialogCloseRequest>,
    /// Action row content, pinned to the bottom of the panel.
    #[prop(optional, into)]
    actions: Option<ViewFn>,
    #[prop(optional)] children: Option<Children>,
) -> impl IntoView {
    let dialog_ref = NodeRef::<leptos::html::Dialog>::new();
    let panel_ref = NodeRef::<leptos::html::Section>::new();
    let close_ref = NodeRef::<leptos::html::Button>::new();
    let invoker = StoredValue::new_local(None::<web_sys::HtmlElement>);

    let title_id = format!("{id}-title");
    let description_id = format!("{id}-description");
    let described_by = description.is_some().then(|| description_id.clone());
    let busy = Signal::derive(move || busy.map(|busy| busy.get()).unwrap_or(false));

    let close_button_visible = dismissal.close_button.is_visible();
    let close_button_locked =
        Signal::derive(move || dismissal.close_button.locks_while_busy() && busy.get());

    let request_close = move |source: DialogCloseSource| {
        let action = resolve_close(policy, dismissal, busy.get_untracked(), source);
        on_close.run(DialogCloseRequest { source, action });
    };

    let apply_initial_focus = move || {
        let requested = initial_focus.requested_element();
        let close_element: Option<web_sys::HtmlElement> = {
            use wasm_bindgen::JsCast;
            close_ref.get_untracked().map(|node| node.unchecked_into())
        };
        let candidate = match initial_focus.kind() {
            DialogInitialFocusKind::CloseButton => close_element.clone(),
            _ => requested,
        };
        let present = candidate.is_some();
        let disabled = candidate.as_ref().map(element_is_disabled).unwrap_or(false);
        match initial_focus_target(initial_focus.kind(), present, disabled) {
            DialogFocusTarget::Requested | DialogFocusTarget::CloseButton => {
                if let Some(element) = candidate {
                    let _ = element.focus();
                }
            }
            DialogFocusTarget::Panel => {
                if let Some(panel) = panel_ref.get_untracked() {
                    use wasm_bindgen::JsCast;
                    let element: web_sys::HtmlElement = panel.unchecked_into();
                    let _ = element.focus();
                }
            }
        }
    };

    let restore_focus = move || {
        // Native `<dialog>` already restores focus to the invoker. Only step in
        // when the invoker no longer exists, so keyboard users never land on
        // the document body.
        let stored = invoker.get_value();
        let connected = stored
            .as_ref()
            .map(|element| element.is_connected())
            .unwrap_or(false);
        match focus_restoration_target(connected) {
            DialogFocusRestoration::Invoker => {
                if let Some(element) = stored {
                    let already = active_element()
                        .map(|active| active == element)
                        .unwrap_or(false);
                    if !already {
                        let _ = element.focus();
                    }
                }
            }
            DialogFocusRestoration::RouteFallback => {
                if let Some(fallback) = route_fallback_element() {
                    let _ = fallback.focus();
                }
            }
        }
        invoker.set_value(None);
    };

    // Whether this dialog currently holds a share of the background scroll
    // lock. Tracked per dialog instance so the release is exact even if the
    // component unmounts while open.
    let holds_background = StoredValue::new(false);

    let release_background = move || {
        if holds_background.get_value() {
            holds_background.set_value(false);
            crate::ui_v2::components::modal_background::release();
        }
    };

    Effect::new(move |_| {
        let is_open = open.get();
        let Some(dialog) = dialog_ref.get() else {
            return;
        };
        if is_open && !dialog.open() {
            // Nothing transient may stay open behind the modal scrim.
            crate::ui_v2::components::transient::notify_modal_opened();
            invoker.set_value(active_element());
            let _ = dialog.show_modal();
            if !holds_background.get_value() {
                holds_background.set_value(true);
                crate::ui_v2::components::modal_background::acquire();
            }
            apply_initial_focus();
        } else if !is_open && dialog.open() {
            dialog.close();
            release_background();
            restore_focus();
        }
    });

    // A route change can unmount an open dialog without the close path ever
    // running. Without this the document would stay scroll-locked with no
    // dialog on screen.
    on_cleanup(release_background);

    let panel_class = format!("arc-dialog-panel arc-clipped-panel {}", width.class());
    let destructive = Signal::derive(move || {
        tone.map(|tone| tone.get())
            .unwrap_or_default()
            .is_destructive()
    });

    view! {
        <dialog
            node_ref=dialog_ref
            class="arc-dialog"
            class:arc-dialog-destructive=move || destructive.get()
            aria-labelledby=title_id.clone()
            aria-describedby=described_by
            on:cancel=move |event: web_sys::Event| {
                // The `open` signal stays authoritative in every case.
                event.prevent_default();
                request_close(DialogCloseSource::Escape);
            }
            on:click=move |_| request_close(DialogCloseSource::Backdrop)
        >
            <section
                node_ref=panel_ref
                class=panel_class
                tabindex="-1"
                on:click=move |event: leptos::ev::MouseEvent| event.stop_propagation()
            >
                <header class="arc-dialog-header">
                    <div class="arc-dialog-heading">
                        {kicker.map(|kicker| view! {
                            <p class="arc-dialog-kicker">{move || kicker.get()}</p>
                        })}
                        <h2
                            id=title_id.clone()
                            class="arc-dialog-title"
                            role=title_live.then_some("status")
                            aria-live=title_live.then_some("polite")
                        >{move || title.get()}</h2>
                    </div>
                    {close_button_visible.then(|| view! {
                        <button
                            node_ref=close_ref
                            type="button"
                            class="arc-dialog-close"
                            aria-label=close_label.clone().unwrap_or_else(|| "Close dialog".to_string())
                            title=move || close_blocked_hint
                                .filter(|_| close_button_locked.get())
                                .map(|hint| hint.get())
                            disabled=move || close_button_locked.get()
                            on:click=move |_| request_close(DialogCloseSource::CloseButton)
                        >
                            <span class="material-symbols-outlined" aria-hidden="true">"close"</span>
                        </button>
                    })}
                </header>
                {description.map(|description| view! {
                    <p id=description_id class="arc-dialog-description">{move || description.get()}</p>
                })}
                {close_blocked_hint.map(|hint| view! {
                    <Show when=move || close_button_locked.get()>
                        <p class="arc-dialog-blocked-hint" role="status" aria-live="polite">
                            {move || hint.get()}
                        </p>
                    </Show>
                })}
                {children.map(|children| view! {
                    <div class="arc-dialog-body">{children()}</div>
                })}
                {actions.map(|actions| view! {
                    <div class="arc-dialog-actions">{actions.run()}</div>
                })}
            </section>
        </dialog>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EVERY_SOURCE: [DialogCloseSource; 6] = [
        DialogCloseSource::Escape,
        DialogCloseSource::Backdrop,
        DialogCloseSource::CloseButton,
        DialogCloseSource::Cancel,
        DialogCloseSource::ConfirmedCancel,
        DialogCloseSource::Completion,
    ];

    #[test]
    fn dismissible_policy_closes_on_every_wired_source() {
        let dismissal = DialogDismissal::freely_dismissible();
        for source in [
            DialogCloseSource::Escape,
            DialogCloseSource::Backdrop,
            DialogCloseSource::CloseButton,
            DialogCloseSource::Cancel,
        ] {
            assert_eq!(
                resolve_close(DialogClosePolicy::Dismissible, dismissal, false, source),
                DialogCloseAction::Dismiss,
                "{source:?} should dismiss"
            );
        }
    }

    #[test]
    fn unwired_sources_are_ignored_regardless_of_policy() {
        let dismissal = DialogDismissal::blocked();
        for source in [
            DialogCloseSource::Escape,
            DialogCloseSource::Backdrop,
            DialogCloseSource::CloseButton,
        ] {
            assert_eq!(
                resolve_close(DialogClosePolicy::Dismissible, dismissal, false, source),
                DialogCloseAction::Ignore,
                "{source:?} is not wired"
            );
        }
    }

    #[test]
    fn escape_policy_is_independent_of_backdrop_policy() {
        let dismissal = DialogDismissal::keyboard_and_button();
        assert_eq!(
            resolve_close(
                DialogClosePolicy::Dismissible,
                dismissal,
                false,
                DialogCloseSource::Escape
            ),
            DialogCloseAction::Dismiss
        );
        assert_eq!(
            resolve_close(
                DialogClosePolicy::Dismissible,
                dismissal,
                false,
                DialogCloseSource::Backdrop
            ),
            DialogCloseAction::Ignore
        );
    }

    #[test]
    fn close_button_is_ignored_when_the_control_is_not_rendered() {
        let dismissal = DialogDismissal {
            escape: DialogSourcePolicy::Allowed,
            backdrop: DialogSourcePolicy::Ignored,
            close_button: DialogCloseButtonPolicy::Hidden,
        };
        assert_eq!(
            resolve_close(
                DialogClosePolicy::Dismissible,
                dismissal,
                false,
                DialogCloseSource::CloseButton
            ),
            DialogCloseAction::Ignore
        );
    }

    #[test]
    fn busy_blocked_policy_suppresses_dismissal_while_busy() {
        let dismissal = DialogDismissal::busy_aware();
        for source in [
            DialogCloseSource::Escape,
            DialogCloseSource::CloseButton,
            DialogCloseSource::Cancel,
        ] {
            assert_eq!(
                resolve_close(DialogClosePolicy::BlockedWhileBusy, dismissal, true, source),
                DialogCloseAction::Ignore
            );
            assert_eq!(
                resolve_close(
                    DialogClosePolicy::BlockedWhileBusy,
                    dismissal,
                    false,
                    source
                ),
                DialogCloseAction::Dismiss
            );
        }
    }

    #[test]
    fn confirm_required_policy_asks_only_while_busy() {
        let dismissal = DialogDismissal::busy_aware();
        assert_eq!(
            resolve_close(
                DialogClosePolicy::ConfirmRequired,
                dismissal,
                true,
                DialogCloseSource::Escape
            ),
            DialogCloseAction::RequestConfirmation
        );
        assert_eq!(
            resolve_close(
                DialogClosePolicy::ConfirmRequired,
                dismissal,
                false,
                DialogCloseSource::Escape
            ),
            DialogCloseAction::Dismiss
        );
    }

    #[test]
    fn non_dismissible_policy_refuses_every_user_channel() {
        let dismissal = DialogDismissal::freely_dismissible();
        for source in EVERY_SOURCE {
            let action = resolve_close(DialogClosePolicy::NonDismissible, dismissal, true, source);
            match source {
                DialogCloseSource::Completion => assert_eq!(action, DialogCloseAction::Dismiss),
                DialogCloseSource::ConfirmedCancel => {
                    assert_eq!(action, DialogCloseAction::CancelOperation)
                }
                _ => assert_eq!(action, DialogCloseAction::Ignore, "{source:?}"),
            }
        }
    }

    #[test]
    fn completion_always_closes_even_when_busy_and_unwired() {
        assert_eq!(
            resolve_close(
                DialogClosePolicy::NonDismissible,
                DialogDismissal::blocked(),
                true,
                DialogCloseSource::Completion
            ),
            DialogCloseAction::Dismiss
        );
    }

    #[test]
    fn confirmed_cancel_routes_to_the_owner_cancellation_flow() {
        assert_eq!(
            resolve_close(
                DialogClosePolicy::ConfirmRequired,
                DialogDismissal::busy_aware(),
                true,
                DialogCloseSource::ConfirmedCancel
            ),
            DialogCloseAction::CancelOperation
        );
    }

    #[test]
    fn initial_focus_falls_back_to_the_panel_when_the_control_is_missing() {
        assert_eq!(
            initial_focus_target(DialogInitialFocusKind::Requested, false, false),
            DialogFocusTarget::Panel
        );
        assert_eq!(
            initial_focus_target(DialogInitialFocusKind::CloseButton, false, false),
            DialogFocusTarget::Panel
        );
    }

    #[test]
    fn initial_focus_never_lands_on_a_disabled_control() {
        assert_eq!(
            initial_focus_target(DialogInitialFocusKind::Requested, true, true),
            DialogFocusTarget::Panel
        );
        assert_eq!(
            initial_focus_target(DialogInitialFocusKind::CloseButton, true, true),
            DialogFocusTarget::Panel
        );
        assert_eq!(
            initial_focus_target(DialogInitialFocusKind::Requested, true, false),
            DialogFocusTarget::Requested
        );
    }

    #[test]
    fn focus_restoration_prefers_the_invoker_and_falls_back_to_the_route() {
        assert_eq!(
            focus_restoration_target(true),
            DialogFocusRestoration::Invoker
        );
        assert_eq!(
            focus_restoration_target(false),
            DialogFocusRestoration::RouteFallback
        );
    }

    #[test]
    fn destructive_tone_does_not_change_dismissal() {
        // Tone is presentation only; the policy table is tone-independent.
        assert!(DialogTone::Destructive.is_destructive());
        assert!(!DialogTone::Neutral.is_destructive());
        assert_eq!(
            resolve_close(
                DialogClosePolicy::Dismissible,
                DialogDismissal::keyboard_and_button(),
                false,
                DialogCloseSource::Escape
            ),
            DialogCloseAction::Dismiss
        );
    }
}
