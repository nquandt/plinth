//! The Plinth UI runtime: the semantic tree, the op applier, the adaptive
//! shell and the control library on gpui-ce (SPEC.md §6 and §9.3).

mod render;
mod theme;
pub mod tree;

pub use render::{GuestPort, PlinthRoot};
pub use theme::{Tokens, WidthClass};

/// Registers the key bindings that the controls need. Call it one time at
/// application start.
pub fn init(cx: &mut gpui::App) {
    use gpui_elements::editable_text::actions::{DEFAULT_INPUT_CONTEXT, default_bindings};
    cx.bind_keys(default_bindings().as_keybindings(Some(DEFAULT_INPUT_CONTEXT)));
}
