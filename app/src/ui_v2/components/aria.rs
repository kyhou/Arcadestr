//! Serialization helpers for enumerated ARIA state attributes.
//!
//! Leptos renders a `bool` attribute value with *boolean attribute* semantics:
//! `true` emits the bare attribute and `false` omits it entirely. That is
//! correct for `disabled` or `hidden`, and wrong for the enumerated ARIA state
//! attributes, where an absent attribute is not the same claim as `"false"`:
//!
//! - absent `aria-expanded` means "this control does not expand anything",
//!   not "collapsed";
//! - absent `aria-pressed` means "this is not a toggle button", not "off";
//! - absent `aria-selected` means "this is not selectable", not "unselected".
//!
//! Binding the raw bool therefore makes collapsed disclosures and inactive
//! toggles advertise no state at all to assistive technology. Bind these
//! helpers instead so both states serialize explicitly.
//!
//! `aria-busy`, `aria-invalid`, and `aria-hidden` are deliberately excluded:
//! their specified default *is* `false`, so omission carries the right meaning.

/// Serialize an ARIA state flag as the explicit string `"true"` or `"false"`.
pub fn aria_bool(state: bool) -> &'static str {
    if state {
        "true"
    } else {
        "false"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_states_serialize_explicitly() {
        assert_eq!(aria_bool(true), "true");
        assert_eq!(aria_bool(false), "false");
    }

    #[test]
    fn the_collapsed_state_is_never_an_empty_string() {
        // A regression here would reintroduce the omitted-attribute defect by
        // another route: an empty value is also not a valid enumerated state.
        assert!(!aria_bool(false).is_empty());
    }
}
