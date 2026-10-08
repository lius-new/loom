//! The editing core: document text, queries, edits, selections and history.
//!
//! * `buffer`    — the text, its line index and the queries behaviours use.
//! * `change`    — requested edits and the change sets they produce.
//! * `editor`    — `EditorMut`, the single entry that changes a document.
//! * `history`   — undo steps, typing runs and explicit groups.
//! * `lines`     — line starts and display widths kept across edits.
//! * `selection` — one view's anchor and active end.
//!
//! Behaviours (default editing, Vim) decide what a key means and express it
//! as edits and selections here; nothing else touches the text.

mod buffer;
mod change;
mod editor;
mod history;
mod lines;
mod selection;

pub use buffer::TextBuffer;
pub use change::{Assoc, Edit};
pub use editor::{Editor, EditorMut};
pub use history::EditKind;
pub use lines::display_width;
pub use selection::Selection;
