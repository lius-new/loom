//! Vim editing behaviour.
//!
//! The Vim layer interprets keys (modes, counts, operators, motions, text
//! objects, registers, repeat) and carries out every change through the
//! editing core (`model::text`). It never stores text of its own: the
//! document, its history and each view's selection stay in the core. Vim's
//! visual selection lives in the view's core selection, with the anchor and
//! the cursor on the first and last selected characters.
//!
//! State that Vim keeps is per active view (mode, pending keys, insert
//! session, command line) or shared by all views (registers, last search,
//! last change, macros). Leaving a view ends its insert session and returns
//! it to Normal mode.

mod command;
mod ex;
mod exec;
mod insert;
pub mod key;
mod motion;
mod object;
mod register;
mod search;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::ops::Range;

use crate::model::document::FileId;
use crate::model::pane_layout::PaneId;
use crate::model::text::{EditorMut, Selection, TextBuffer};

use command::{Command, FindKind, InsertKind, Operator, ScrollTo};
pub use key::Key;
pub use register::Clipboard;
use register::Registers;

/// One editor view: a document shown in a pane.
pub type ViewId = (PaneId, FileId);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VisualKind {
    Char,
    Line,
    Block,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Normal,
    Insert,
    Replace,
    Visual(VisualKind),
}

impl Mode {
    /// The `mode` attribute of the key context while this mode is active.
    pub fn context_name(self) -> &'static str {
        match self {
            Mode::Normal => "normal",
            Mode::Insert => "insert",
            Mode::Replace => "replace",
            Mode::Visual(_) => "visual",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Mode::Normal => "NORMAL",
            Mode::Insert => "INSERT",
            Mode::Replace => "REPLACE",
            Mode::Visual(VisualKind::Char) => "VISUAL",
            Mode::Visual(VisualKind::Line) => "VISUAL LINE",
            Mode::Visual(VisualKind::Block) => "VISUAL BLOCK",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorShape {
    Block,
    Bar,
    Underline,
    /// Operator pending: a lower half block.
    HalfBlock,
}

/// Application operations Vim asks the host to perform.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Request {
    Save,
    Quit { force: bool },
    SaveAndQuit,
    Scroll(ScrollPosition),
}

/// Where `zz`, `zt` and `zb` put the cursor line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollPosition {
    Center,
    Top,
    Bottom,
}

impl From<ScrollTo> for ScrollPosition {
    fn from(value: ScrollTo) -> Self {
        match value {
            ScrollTo::Center => ScrollPosition::Center,
            ScrollTo::Top => ScrollPosition::Top,
            ScrollTo::Bottom => ScrollPosition::Bottom,
        }
    }
}

/// What Vim needs from the application around it.
pub trait Host: Clipboard {
    /// Lines visible in the editor.
    fn page_lines(&self) -> usize;
    /// The range of lines currently on screen.
    fn visible_lines(&self) -> Range<usize>;
    fn request(&mut self, request: Request);
}

/// User preferences for the Vim layer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Options {
    pub enabled: bool,
    /// Unnamed register yanks, deletes and puts go through the clipboard.
    pub system_clipboard: bool,
    pub ignore_case: bool,
    pub smart_case: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    pub text: String,
    pub error: bool,
}

/// What the status bar shows for Vim.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    pub mode: &'static str,
    /// Keys of a command still being typed (`showcmd`).
    pub pending: String,
    /// The command line being typed, with its `:`, `/` or `?`.
    pub command_line: Option<String>,
    pub message: Option<Message>,
    pub recording: Option<char>,
}

/// An edit made while typing in Insert or Replace mode, kept so the insert
/// can be repeated with `.`, a count, or across a visual block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InsertEvent {
    Text(String),
    Backspace,
    Delete,
    DeleteWordBack,
    DeleteWordForward,
    DeleteToLineStart,
    Newline,
    Tab,
    Backtab,
    Paste(String),
}

#[derive(Clone, Debug)]
struct InsertSession {
    /// The command that started the insert, repeated by `.`; `None` once
    /// the cursor moved away from where typing started.
    repeat: Option<Command>,
    events: Vec<InsertEvent>,
    /// `3ihi<Esc>` types "hi" three times.
    count: usize,
    /// `o` and `O` open a new line for each repetition.
    kind: Option<InsertKind>,
    /// A visual block insert, replicated onto the block's other lines.
    block: Option<BlockInsert>,
    visual: Option<VisualExtent>,
}

#[derive(Clone, Copy, Debug)]
struct BlockInsert {
    /// The block's lines after the first one.
    lines: (usize, usize),
    column: usize,
    /// `A` appends after each line's block; `$` blocks append at line ends.
    to_line_end: bool,
}

/// The size of a visual selection, so `.` can repeat a visual operator on
/// the same amount of text at the cursor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct VisualExtent {
    kind: VisualKind,
    lines: usize,
    /// For one-line character selections, its length in characters; for
    /// blocks, the width in display columns.
    width: usize,
    to_line_end: bool,
}

/// The last change, for `.`.
#[derive(Clone, Debug)]
struct Change {
    command: Command,
    /// Typed text when the change entered Insert mode.
    insert: Option<Vec<InsertEvent>>,
    visual: Option<VisualExtent>,
}

#[derive(Clone, Debug)]
struct CommandLine {
    prefix: char,
    text: String,
    /// For `/` and `?` typed after an operator or with a count.
    search: Option<PendingSearch>,
    /// Lines of the visual selection `:` was typed in, for `'<,'>`.
    visual_lines: Option<(usize, usize)>,
}

#[derive(Clone, Copy, Debug)]
struct PendingSearch {
    register: Option<char>,
    count: Option<usize>,
    operator: Option<Operator>,
    forward: bool,
}

#[derive(Clone, Debug)]
struct LastSearch {
    pattern: String,
    forward: bool,
}

/// The last visual selection, for `gv`.
#[derive(Clone, Copy, Debug)]
struct LastVisual {
    document: FileId,
    kind: VisualKind,
    anchor: usize,
    cursor: usize,
}

#[derive(Clone, Debug, Default)]
pub struct Vim {
    pub options: Options,
    mode: Mode,
    view: Option<ViewId>,
    /// Keys of the Normal or Visual mode command being typed.
    keys: Vec<Key>,
    command_line: Option<CommandLine>,
    insert: Option<InsertSession>,
    /// Characters overwritten in Replace mode, restored by backspace.
    replaced: Vec<Option<String>>,
    /// Whether this layer opened a history group that is still open.
    group_open: bool,
    registers: Registers,
    last_find: Option<(FindKind, char)>,
    search: Option<LastSearch>,
    /// Highlight matches of the last search.
    highlight_search: bool,
    last_change: Option<Change>,
    /// Set when a command could not be carried out; stops macro playback.
    failed: bool,
    /// `<C-o>` in Insert mode: run one Normal mode command, then type again.
    one_command: bool,
    message: Option<Message>,
    last_visual: Option<LastVisual>,
    recording: Option<(char, Vec<Key>)>,
    last_macro: Option<char>,
    macro_depth: usize,
    /// Marks `a`-`z` per document, `'` (the jump before the last) included.
    marks: HashMap<(FileId, char), usize>,
    jumps: Vec<(FileId, usize)>,
    jump_index: usize,
}

impl Vim {
    pub fn is_enabled(&self) -> bool {
        self.options.enabled
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// The view the current mode belongs to.
    pub fn view(&self) -> Option<ViewId> {
        self.view
    }

    /// Whether text input belongs to Vim's typing modes, so the input method
    /// may compose: Insert, Replace and the command line.
    pub fn accepts_text(&self) -> bool {
        self.command_line.is_some() || matches!(self.mode, Mode::Insert | Mode::Replace)
    }

    pub fn is_inserting(&self) -> bool {
        matches!(self.mode, Mode::Insert | Mode::Replace) && self.command_line.is_none()
    }

    pub fn cursor_shape(&self) -> CursorShape {
        if self.command_line.is_some() {
            return CursorShape::Block;
        }
        match self.mode {
            Mode::Insert => CursorShape::Bar,
            Mode::Replace => CursorShape::Underline,
            _ if self.operator_pending() => CursorShape::HalfBlock,
            _ => CursorShape::Block,
        }
    }

    fn operator_pending(&self) -> bool {
        self.keys
            .iter()
            .any(|key| matches!(key, Key::Char('d' | 'c' | 'y' | '<' | '>')))
    }

    pub fn status(&self) -> Status {
        Status {
            mode: self.mode.label(),
            pending: self.keys.iter().map(ToString::to_string).collect(),
            command_line: self
                .command_line
                .as_ref()
                .map(|line| format!("{}{}", line.prefix, line.text)),
            message: self.message.clone(),
            recording: self.recording.as_ref().map(|(name, _)| *name),
        }
    }

    pub fn show_message(&mut self, text: impl Into<String>, error: bool) {
        self.message = Some(Message {
            text: text.into(),
            error,
        });
    }

    /// Turn the layer on or off. Turning it off ends any insert session and
    /// visual selection so the default editing behaviour takes over cleanly.
    pub fn set_enabled(&mut self, enabled: bool, editor: Option<&mut EditorMut<'_>>) {
        if self.options.enabled == enabled {
            return;
        }
        self.leave_view(editor);
        self.options.enabled = enabled;
        self.message = None;
    }

    /// Make `view` the active one. Called before Vim handles input for it; a
    /// different view first leaves the previous one.
    pub fn enter_view(&mut self, view: ViewId, editor: &mut EditorMut<'_>) {
        if self.view == Some(view) {
            return;
        }
        self.view = Some(view);
        self.mode = Mode::Normal;
        self.keys.clear();
        self.command_line = None;
        self.adopt_selection(editor);
    }

    /// The active view loses its Vim state: an insert session ends (its undo
    /// step closes) and the view returns to Normal mode.
    pub fn leave_view(&mut self, editor: Option<&mut EditorMut<'_>>) {
        if let Some(editor) = editor {
            match self.mode {
                Mode::Insert | Mode::Replace => self.finish_insert(editor, false),
                Mode::Visual(_) => self.exit_visual(editor),
                Mode::Normal => {}
            }
            if self.group_open {
                editor.end_group();
            }
        }
        self.group_open = false;
        self.insert = None;
        self.mode = Mode::Normal;
        self.keys.clear();
        self.command_line = None;
        self.view = None;
    }

    /// A pointer click or drag changed the selection: cancel any pending
    /// command and follow the selection into Visual or Normal mode.
    pub fn after_pointer(&mut self, editor: &mut EditorMut<'_>) {
        self.keys.clear();
        self.command_line = None;
        match self.mode {
            Mode::Insert | Mode::Replace => {
                // Typing continues at the new place; `.` repeats only what is
                // typed from here on.
                if let Some(session) = self.insert.as_mut() {
                    session.repeat = Some(insert_command(InsertKind::Before));
                    session.events.clear();
                    session.count = 1;
                    session.kind = None;
                    session.block = None;
                }
                self.replaced.clear();
            }
            Mode::Normal | Mode::Visual(_) => {
                self.mode = Mode::Normal;
                self.adopt_selection(editor);
            }
        }
    }

    /// Take over a selection made outside Vim (pointer, select all): a range
    /// becomes a character-wise visual selection; otherwise the cursor is
    /// placed on a character.
    pub fn adopt_selection(&mut self, editor: &mut EditorMut<'_>) {
        let selection = editor.selection_state();
        if self.mode != Mode::Normal {
            return;
        }
        match selection.anchor {
            Some(anchor) if anchor != selection.cursor => {
                let (anchor, cursor) = if selection.cursor > anchor {
                    (anchor, editor.prev_grapheme(selection.cursor))
                } else {
                    (editor.prev_grapheme(anchor), selection.cursor)
                };
                self.mode = Mode::Visual(VisualKind::Char);
                editor.set_selection(Selection::spanning(anchor, cursor));
            }
            _ => {
                let cursor = motion::normal_position(editor, selection.cursor);
                if selection.anchor.is_some() || cursor != selection.cursor {
                    editor.set_selection(Selection {
                        cursor,
                        anchor: None,
                        preferred_column: selection.preferred_column,
                    });
                }
            }
        }
    }

    /// Handle one key for the active view. Returns whether Vim consumed it;
    /// an unconsumed key continues to the default text input.
    pub fn handle_key(
        &mut self,
        editor: &mut EditorMut<'_>,
        key: Key,
        host: &mut dyn Host,
    ) -> bool {
        if self.command_line.is_some() {
            self.record_key(key);
            self.message = None;
            self.command_line_key(editor, key, host);
            return true;
        }
        match self.mode {
            Mode::Insert | Mode::Replace => {
                // Keys typed as text are recorded when their text arrives.
                let consumed = self.insert_key(editor, key, host);
                if consumed {
                    self.record_key(key);
                }
                consumed
            }
            Mode::Normal | Mode::Visual(_) => {
                self.record_key(key);
                self.message = None;
                if self.mode == Mode::Normal {
                    self.adopt_selection(editor);
                }
                self.keys.push(key);
                self.process_keys(editor, host);
                true
            }
        }
    }

    /// Handle committed text. Returns whether Vim used it; in Normal and
    /// Visual mode text is not a command and is left to the caller.
    pub fn handle_text(&mut self, editor: &mut EditorMut<'_>, text: &str) -> bool {
        if let Some(line) = self.command_line.as_mut() {
            line.text.push_str(&text.replace(['\r', '\n'], ""));
            return true;
        }
        if !self.is_inserting() {
            return false;
        }
        if let Some((_, keys)) = self.recording.as_mut() {
            keys.extend(text.chars().map(Key::Char));
        }
        self.insert_event(editor, InsertEvent::Text(text.to_owned()));
        true
    }

    /// Record an edit the default editing behaviour made in Insert mode (a
    /// paste or word deletion), so `.` and counts repeat it. A cursor move
    /// starts a new repeatable insert at the new place.
    /// Keys sent by a key mapping, handled as if typed but not mapped again.
    pub fn feed_keys(&mut self, editor: &mut EditorMut<'_>, keys: &[Key], host: &mut dyn Host) {
        for &key in keys {
            self.feed(editor, host, key);
        }
    }

    pub fn record_insert(&mut self, event: Option<InsertEvent>) {
        let Some(session) = self.insert.as_mut() else {
            return;
        };
        match event {
            Some(event) => session.events.push(event),
            None => {
                session.repeat = Some(insert_command(InsertKind::Before));
                session.events.clear();
                session.count = 1;
                session.kind = None;
                session.block = None;
            }
        }
    }

    /// Ranges to highlight instead of the core selection, in Visual mode.
    pub fn highlights(
        &self,
        buffer: &TextBuffer,
        selection: Selection,
    ) -> Option<Vec<Range<usize>>> {
        let Mode::Visual(kind) = self.mode else {
            return None;
        };
        Some(exec::visual_ranges(buffer, selection, kind))
    }

    /// The selected text in Visual mode, for the application's Copy.
    pub fn visual_text(&self, buffer: &TextBuffer, selection: Selection) -> Option<String> {
        let Mode::Visual(kind) = self.mode else {
            return None;
        };
        let separator = buffer.line_ending();
        Some(
            exec::visual_ranges(buffer, selection, kind)
                .into_iter()
                .map(|range| &buffer.text()[range])
                .collect::<Vec<_>>()
                .join(if kind == VisualKind::Block {
                    separator
                } else {
                    ""
                }),
        )
    }

    /// Search matches to highlight on lines `lines`, after a search.
    pub fn search_matches(&self, buffer: &TextBuffer, lines: Range<usize>) -> Vec<Range<usize>> {
        let Some(search) = self.search.as_ref().filter(|_| self.highlight_search) else {
            return Vec::new();
        };
        let Ok(regex) = search::compile(
            &search.pattern,
            self.options.ignore_case,
            self.options.smart_case,
        ) else {
            return Vec::new();
        };
        if lines.is_empty() {
            return Vec::new();
        }
        let start = buffer.line_start(lines.start);
        let end = buffer.line_range_with_break(lines.end - 1).end;
        regex
            .find_iter(&buffer.text()[start..end])
            .filter(|found| !found.is_empty())
            .map(|found| start + found.start()..start + found.end())
            .collect()
    }

    fn record_key(&mut self, key: Key) {
        if let Some((_, keys)) = self.recording.as_mut() {
            keys.push(key);
        }
    }
}

fn insert_command(kind: InsertKind) -> Command {
    Command {
        register: None,
        count: None,
        action: command::Action::Simple(command::Simple::Insert(kind)),
    }
}
