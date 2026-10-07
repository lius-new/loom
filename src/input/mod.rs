//! Input layer: keystrokes, key contexts, the keymap and sequence dispatch.
//!
//! The design follows Zed's keymap (see `KEYMAP_PLAN.md`): keymap files bind
//! keystroke sequences to named actions per context, and the deepest matching
//! context wins.

pub mod action;
pub mod context;
pub mod context_stack;
pub mod dispatcher;
pub mod keymap;
pub mod keymap_file;
pub mod keystroke;
