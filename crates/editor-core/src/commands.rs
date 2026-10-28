//! Semantic editing commands; platform keys and clipboard I/O belong to the host.
use crate::normal::{self, Movement};
use crate::text::EditorMut;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Move(Movement, bool),
    Delete(bool, bool),
    Enter,
    Indent(bool),
    Line(normal::LineOperation),
    ToggleComment,
    SelectAll,
    Escape,
    Undo,
    Redo,
    Copy,
    Cut,
    Paste,
}
// Preserve host-side specialization for a known command across the crate boundary.
#[inline]
pub fn apply(editor: &mut EditorMut<'_>, command: Command) {
    match command {
        Command::Move(movement, extend) => normal::navigate(editor, movement, extend),
        Command::Delete(forward, word) => normal::erase(editor, forward, word),
        Command::Enter => normal::enter(editor),
        Command::Indent(outdent) => normal::indent(editor, outdent),
        Command::Line(operation) => normal::lines(editor, operation),
        Command::ToggleComment => normal::comment(editor, "//"),
        Command::SelectAll => editor.select_all(),
        Command::Escape => editor.clear_selection(),
        Command::Undo => {
            editor.undo();
        }
        Command::Redo => {
            editor.redo();
        }
        Command::Copy | Command::Cut | Command::Paste => {}
    }
}
