//! Carrying out Normal and Visual mode commands through the editing core.

use std::ops::Range;

use crate::model::text::{
    Assoc, Edit, EditKind, EditorMut, Selection, TextBuffer, display_width, grapheme_width,
};

use super::command::{
    self, Action, Command, Context, InsertKind, Motion, Operator, Parsed, Simple, Target,
    TextObject,
};
use super::motion::{self, MotionKind, Moved, first_non_blank, last_char, normal_position};
use super::object::{self, Selected};
use super::register::{Register, RegisterKind, Store};
use super::search;
use super::{
    Change, CommandLine, Host, InsertSession, Key, LastSearch, LastVisual, Mode, PendingSearch,
    Request, Vim, VisualExtent, VisualKind,
};

/// Macro playback nested deeper than this stops, so a register that plays
/// itself can not run away.
const MAX_MACRO_DEPTH: usize = 20;

/// The text an operator acts on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Region {
    Chars(Range<usize>),
    /// First and last line.
    Lines(usize, usize),
    /// Lines and display columns; `right` is `usize::MAX` for `$` blocks.
    Block {
        first: usize,
        last: usize,
        left: usize,
        right: usize,
    },
}

impl Region {
    fn lines(&self, buffer: &TextBuffer) -> (usize, usize) {
        match *self {
            Region::Chars(ref range) => (
                buffer.line_of(range.start),
                buffer.line_of(range.end.saturating_sub(1).max(range.start)),
            ),
            Region::Lines(first, last) | Region::Block { first, last, .. } => (first, last),
        }
    }
}

/// Display width of the character at `offset`, at least one column.
fn char_width(buffer: &TextBuffer, offset: usize) -> usize {
    buffer
        .grapheme_at(offset)
        .filter(|grapheme| !grapheme.starts_with(['\r', '\n']))
        .map_or(1, |grapheme| {
            grapheme_width(grapheme, buffer.display_column(offset)).max(1)
        })
}

/// The region a visual selection covers. The selection's anchor and cursor
/// are on the first and last selected characters.
pub(super) fn visual_region(buffer: &TextBuffer, selection: Selection, kind: VisualKind) -> Region {
    let anchor = selection.anchor.unwrap_or(selection.cursor);
    let cursor = selection.cursor;
    let (start, end) = (anchor.min(cursor), anchor.max(cursor));
    let to_line_end = selection.preferred_column == Some(usize::MAX);
    match kind {
        VisualKind::Char => {
            let end = if to_line_end && end == cursor {
                buffer.line_range_with_break(buffer.line_of(end)).end
            } else if end < buffer.len() {
                buffer.next_grapheme(end)
            } else {
                end
            };
            Region::Chars(start..end)
        }
        VisualKind::Line => Region::Lines(buffer.line_of(start), buffer.line_of(end)),
        VisualKind::Block => {
            let (anchor_column, cursor_column) =
                (buffer.display_column(anchor), buffer.display_column(cursor));
            let right = if to_line_end {
                usize::MAX
            } else {
                (anchor_column + char_width(buffer, anchor))
                    .max(cursor_column + char_width(buffer, cursor))
            };
            Region::Block {
                first: buffer.line_of(start),
                last: buffer.line_of(end),
                left: anchor_column.min(cursor_column),
                right,
            }
        }
    }
}

pub(super) fn visual_ranges(
    buffer: &TextBuffer,
    selection: Selection,
    kind: VisualKind,
) -> Vec<Range<usize>> {
    region_ranges(buffer, &visual_region(buffer, selection, kind))
}

/// The ranges of text a region covers, one per line for blocks.
fn region_ranges(buffer: &TextBuffer, region: &Region) -> Vec<Range<usize>> {
    match *region {
        Region::Chars(ref range) => vec![range.clone()],
        Region::Lines(first, last) => {
            let lines = buffer.line_start(first)..buffer.line_range_with_break(last).end;
            vec![lines]
        }
        Region::Block {
            first,
            last,
            left,
            right,
        } => (first..=last)
            .filter_map(|line| buffer.display_column_range(line, left, right))
            .collect(),
    }
}

/// The register an operator fills from `region`.
fn region_register(buffer: &TextBuffer, region: &Region) -> Register {
    match *region {
        Region::Chars(ref range) => {
            Register::new(&buffer.text()[range.clone()], RegisterKind::Char)
        }
        Region::Lines(first, last) => {
            let mut text = (first..=last)
                .map(|line| buffer.line(line))
                .collect::<Vec<_>>()
                .join("\n");
            text.push('\n');
            Register::new(text, RegisterKind::Line)
        }
        Region::Block {
            first,
            last,
            left,
            right,
        } => {
            let pieces: Vec<&str> = (first..=last)
                .map(|line| {
                    buffer
                        .display_column_range(line, left, right)
                        .map_or("", |range| &buffer.text()[range])
                })
                .collect();
            Register::new(pieces.join("\n"), RegisterKind::Block)
        }
    }
}

/// The end of an inclusive motion ending on `offset`: after its character,
/// but never past the line's content.
fn inclusive_end(buffer: &TextBuffer, offset: usize) -> usize {
    if offset < buffer.line_end(buffer.line_of(offset)) {
        buffer.next_grapheme(offset)
    } else {
        offset
    }
}

/// The region between `from` and a motion's target, with Vim's rules for
/// exclusive motions that end at the start of a line (`:h exclusive`).
fn motion_region(buffer: &TextBuffer, from: usize, moved: Moved) -> Region {
    let (start, end) = (from.min(moved.offset), from.max(moved.offset));
    match moved.kind {
        MotionKind::Linewise => Region::Lines(buffer.line_of(start), buffer.line_of(end)),
        MotionKind::Inclusive => Region::Chars(start..inclusive_end(buffer, end)),
        MotionKind::Exclusive => {
            let (start_line, end_line) = (buffer.line_of(start), buffer.line_of(end));
            if end > start && end_line > start_line && end == buffer.line_start(end_line) {
                let previous = end_line - 1;
                if start <= first_non_blank(buffer, start_line) {
                    return Region::Lines(start_line, previous);
                }
                return Region::Chars(start..buffer.line_end(previous));
            }
            Region::Chars(start..end)
        }
    }
}

fn spaces(count: usize) -> String {
    " ".repeat(count)
}

fn leading_whitespace(buffer: &TextBuffer, line: usize) -> String {
    buffer
        .line(line)
        .chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .collect()
}

/// Escape a literal word for a Vim search pattern.
fn escape_pattern(word: &str) -> String {
    let mut escaped = String::new();
    for c in word.chars() {
        if "\\.*[]~^$/".contains(c) {
            escaped.push('\\');
        }
        escaped.push(c);
    }
    escaped
}

/// Whether the motion is a jump that `''` and `<C-o>` return from.
fn is_jump(motion: &Motion) -> bool {
    matches!(
        motion,
        Motion::FirstLine
            | Motion::LastLine
            | Motion::MatchPair
            | Motion::ParagraphForward
            | Motion::ParagraphBackward
            | Motion::SearchNext { .. }
            | Motion::SearchWord { .. }
            | Motion::Search { .. }
            | Motion::ScreenTop
            | Motion::ScreenMiddle
            | Motion::ScreenBottom
            | Motion::Mark { .. }
    )
}

fn changes_text(command: &Command) -> bool {
    match &command.action {
        Action::Operate(operator, _) => operator.changes_text(),
        Action::Simple(simple) => matches!(
            simple,
            Simple::Insert(_)
                | Simple::DeleteChar { .. }
                | Simple::Substitute
                | Simple::SubstituteLine
                | Simple::DeleteToEnd
                | Simple::ChangeToEnd
                | Simple::Join { .. }
                | Simple::ToggleCaseChar
                | Simple::Put { .. }
                | Simple::ReplaceChar(_)
                | Simple::ReplaceMode
                | Simple::VisualInsert { .. }
                | Simple::Increment(_)
        ),
        _ => false,
    }
}

impl Vim {
    // ---- Dispatch --------------------------------------------------------------

    pub(super) fn process_keys(&mut self, editor: &mut EditorMut<'_>, host: &mut dyn Host) {
        let context = if matches!(self.mode, Mode::Visual(_)) {
            Context::Visual
        } else {
            Context::Normal
        };
        match command::parse(&self.keys, context, self.recording.is_some()) {
            Parsed::Incomplete => {}
            Parsed::Invalid => {
                self.keys.clear();
                self.failed = true;
            }
            Parsed::Done(command) => {
                self.keys.clear();
                self.run(editor, host, command);
            }
            Parsed::Search {
                register,
                count,
                operator,
                forward,
            } => {
                self.keys.clear();
                self.command_line = Some(CommandLine {
                    prefix: if forward { '/' } else { '?' },
                    text: String::new(),
                    search: Some(PendingSearch {
                        register,
                        count,
                        operator,
                        forward,
                    }),
                    visual_lines: None,
                });
            }
        }
        if self.one_command
            && self.keys.is_empty()
            && self.command_line.is_none()
            && self.mode == Mode::Normal
        {
            // `<C-o>` in Insert mode: back to typing after one command.
            self.one_command = false;
            self.mode = Mode::Insert;
            self.record_insert(None);
        }
    }

    /// Run a complete command: one undo step for changes, remembered for `.`.
    pub(super) fn run(
        &mut self,
        editor: &mut EditorMut<'_>,
        host: &mut dyn Host,
        command: Command,
    ) {
        let changes = changes_text(&command);
        if changes && !self.group_open {
            editor.begin_group();
            self.group_open = true;
        }
        let visual = self.visual_extent(editor);
        let version = editor.version();
        self.execute(editor, host, &command);
        if self.is_inserting() {
            if let Some(session) = self.insert.as_mut() {
                session.visual = visual;
            }
            return;
        }
        if self.one_command {
            return;
        }
        if self.group_open {
            editor.end_group();
            self.group_open = false;
        }
        if changes && editor.version() != version {
            self.last_change = Some(Change {
                command,
                insert: None,
                visual,
            });
        }
        self.normalize(editor);
    }

    /// Keep the selection consistent with the mode: a Normal-mode cursor sits
    /// on a character and has no anchor.
    pub(super) fn normalize(&mut self, editor: &mut EditorMut<'_>) {
        let selection = editor.selection_state();
        match self.mode {
            Mode::Normal => {
                let cursor = normal_position(editor, selection.cursor);
                if cursor != selection.cursor || selection.anchor.is_some() {
                    editor.set_selection(Selection {
                        cursor,
                        anchor: None,
                        preferred_column: selection.preferred_column,
                    });
                }
            }
            Mode::Visual(_) if selection.anchor.is_none() => {
                editor.set_selection(Selection {
                    anchor: Some(selection.cursor),
                    ..selection
                });
            }
            _ => {}
        }
    }

    fn execute(&mut self, editor: &mut EditorMut<'_>, host: &mut dyn Host, command: &Command) {
        match &command.action {
            Action::Move(motion) => self.move_cursor(editor, host, motion, command.count),
            Action::Select(object) => self.select_object(editor, *object, command.count),
            Action::Operate(operator, target) => {
                self.operate(editor, host, command, *operator, target)
            }
            Action::Simple(simple) => self.simple(editor, host, command, simple),
        }
    }

    fn set_cursor(&self, editor: &mut EditorMut<'_>, offset: usize) {
        editor.set_selection(Selection::caret(normal_position(editor, offset)));
    }

    // ---- Motions ---------------------------------------------------------------

    fn move_cursor(
        &mut self,
        editor: &mut EditorMut<'_>,
        host: &mut dyn Host,
        motion: &Motion,
        count: Option<usize>,
    ) {
        let selection = editor.selection_state();
        let Some(moved) = self.eval_motion(editor, selection, host, motion, count, None) else {
            self.failed = true;
            return;
        };
        if is_jump(motion) {
            self.push_jump(selection.cursor);
        }
        let visual = matches!(self.mode, Mode::Visual(_));
        let cursor = normal_position(editor, moved.offset);
        let preferred_column = match moved.kind {
            MotionKind::Linewise if moved.column.is_none() => None,
            _ => moved.column,
        };
        editor.set_selection(Selection {
            cursor,
            anchor: if visual {
                Some(selection.anchor.unwrap_or(selection.cursor))
            } else {
                None
            },
            preferred_column,
        });
    }

    /// Where `motion` goes from the selection's cursor, or `None` when it
    /// can not move (which makes an operator do nothing).
    pub(super) fn eval_motion(
        &mut self,
        buffer: &TextBuffer,
        selection: Selection,
        host: &mut dyn Host,
        motion: &Motion,
        count: Option<usize>,
        operator: Option<Operator>,
    ) -> Option<Moved> {
        let cursor = selection.cursor;
        let n = count.unwrap_or(1).max(1);
        let line = buffer.line_of(cursor);
        let last = buffer.line_count() - 1;
        let pending = operator.is_some();
        let column = || {
            selection
                .preferred_column
                .unwrap_or_else(|| buffer.display_column(cursor))
        };
        let vertical = |target: usize| Moved {
            offset: motion::at_column(buffer, target, column()),
            kind: MotionKind::Linewise,
            column: Some(column()),
        };
        Some(match motion {
            Motion::Left => {
                let start = buffer.line_start(line);
                if cursor <= start {
                    return None;
                }
                let mut offset = cursor;
                for _ in 0..n {
                    if offset <= start {
                        break;
                    }
                    offset = buffer.prev_grapheme(offset);
                }
                Moved::exclusive(offset)
            }
            Motion::Right => {
                let limit = if pending {
                    buffer.line_end(line)
                } else {
                    last_char(buffer, line)
                };
                if cursor >= limit {
                    return None;
                }
                let mut offset = cursor;
                for _ in 0..n {
                    if offset >= limit {
                        break;
                    }
                    offset = buffer.next_grapheme(offset).min(limit);
                }
                Moved::exclusive(offset)
            }
            Motion::Up | Motion::DisplayUp => {
                if line == 0 {
                    return None;
                }
                vertical(line.saturating_sub(n))
            }
            Motion::Down | Motion::DisplayDown => {
                if line == last {
                    return None;
                }
                vertical((line + n).min(last))
            }
            Motion::WrapLeft => {
                let mut offset = cursor;
                for _ in 0..n {
                    let here = buffer.line_of(offset);
                    if offset == buffer.line_start(here) {
                        if here == 0 {
                            break;
                        }
                        offset = if pending {
                            buffer.line_end(here - 1)
                        } else {
                            last_char(buffer, here - 1)
                        };
                    } else {
                        offset = buffer.prev_grapheme(offset);
                    }
                }
                if offset == cursor {
                    return None;
                }
                Moved::exclusive(offset)
            }
            Motion::WrapRight => {
                let mut offset = cursor;
                for _ in 0..n {
                    let here = buffer.line_of(offset);
                    let end = if pending {
                        buffer.line_end(here)
                    } else {
                        last_char(buffer, here)
                    };
                    if offset >= end {
                        if here == last {
                            break;
                        }
                        offset = buffer.line_start(here + 1);
                    } else {
                        offset = buffer.next_grapheme(offset);
                    }
                }
                if offset == cursor {
                    return None;
                }
                Moved::exclusive(offset)
            }
            Motion::WordStart { big } => {
                if operator == Some(Operator::Change) && motion::class(buffer, cursor, *big) != 0 {
                    // `cw` changes to the end of the word, like `ce`.
                    Moved::inclusive(motion::word_end(buffer, cursor, n, *big, true))
                } else {
                    let offset = motion::word_forward(buffer, cursor, n, *big, pending);
                    if offset == cursor {
                        return None;
                    }
                    Moved::exclusive(offset)
                }
            }
            Motion::WordBack { big } => {
                let offset = motion::word_back(buffer, cursor, n, *big);
                if offset == cursor {
                    return None;
                }
                Moved::exclusive(offset)
            }
            Motion::WordEnd { big } => {
                Moved::inclusive(motion::word_end(buffer, cursor, n, *big, false))
            }
            Motion::WordEndBack { big } => {
                Moved::inclusive(motion::word_end_back(buffer, cursor, n, *big))
            }
            Motion::LineStart => Moved {
                offset: buffer.line_start(line),
                kind: MotionKind::Exclusive,
                column: Some(0),
            },
            Motion::FirstNonBlank => Moved::exclusive(first_non_blank(buffer, line)),
            Motion::LineEnd => {
                let target = line + n - 1;
                if target > last {
                    return None;
                }
                Moved {
                    offset: last_char(buffer, target),
                    kind: MotionKind::Inclusive,
                    column: Some(usize::MAX),
                }
            }
            Motion::NextLineFirstNonBlank => {
                if line + n > last {
                    return None;
                }
                Moved::linewise(first_non_blank(buffer, line + n))
            }
            Motion::PrevLineFirstNonBlank => {
                if line < n {
                    return None;
                }
                Moved::linewise(first_non_blank(buffer, line - n))
            }
            Motion::CurrentLineFirstNonBlank => {
                Moved::linewise(first_non_blank(buffer, (line + n - 1).min(last)))
            }
            Motion::FirstLine => {
                let target = count.map_or(0, |count| count.saturating_sub(1)).min(last);
                Moved::linewise(first_non_blank(buffer, target))
            }
            Motion::LastLine => {
                let target = count
                    .map_or(last, |count| count.saturating_sub(1))
                    .min(last);
                Moved::linewise(first_non_blank(buffer, target))
            }
            Motion::Find { kind, target } => {
                self.last_find = Some((*kind, *target));
                let offset = motion::find_in_line(buffer, cursor, *kind, *target, n, false)?;
                match kind {
                    command::FindKind::To | command::FindKind::Till => Moved::inclusive(offset),
                    _ => Moved::exclusive(offset),
                }
            }
            Motion::RepeatFind { reverse } => {
                let (kind, target) = self.last_find?;
                let kind = if *reverse { kind.reversed() } else { kind };
                let offset = motion::find_in_line(buffer, cursor, kind, target, n, true)?;
                match kind {
                    command::FindKind::To | command::FindKind::Till => Moved::inclusive(offset),
                    _ => Moved::exclusive(offset),
                }
            }
            Motion::MatchPair => match count {
                Some(percent) => {
                    let target =
                        ((buffer.line_count() * percent.min(100)).div_ceil(100)).saturating_sub(1);
                    Moved::linewise(first_non_blank(buffer, target))
                }
                None => Moved::inclusive(motion::match_pair(buffer, cursor)?),
            },
            Motion::ParagraphForward | Motion::ParagraphBackward => {
                let moved =
                    motion::paragraph(buffer, cursor, n, *motion == Motion::ParagraphForward);
                if moved.offset == cursor {
                    return None;
                }
                moved
            }
            Motion::SearchNext { reverse } => {
                let Some(search) = self.search.clone() else {
                    self.show_message("E35: No previous regular expression", true);
                    return None;
                };
                let forward = search.forward != *reverse;
                Moved::exclusive(self.find_pattern(buffer, cursor, &search.pattern, forward, n)?)
            }
            Motion::SearchWord { forward } => {
                let Some((start, word)) = word_under_cursor(buffer, cursor) else {
                    self.show_message("E348: No string under cursor", true);
                    return None;
                };
                let pattern = format!("\\<{}\\>", escape_pattern(&word));
                self.set_search(pattern.clone(), *forward);
                let from = if *forward { cursor } else { start };
                Moved::exclusive(self.find_pattern(buffer, from, &pattern, *forward, n)?)
            }
            Motion::Search { pattern, forward } => {
                Moved::exclusive(self.find_pattern(buffer, cursor, pattern, *forward, n)?)
            }
            Motion::HalfPageDown | Motion::HalfPageUp | Motion::PageDown | Motion::PageUp => {
                let page = host.page_lines().max(1);
                let lines = match motion {
                    Motion::HalfPageDown | Motion::HalfPageUp => count.unwrap_or((page / 2).max(1)),
                    _ => n * page.saturating_sub(2).max(1),
                };
                let down = matches!(motion, Motion::HalfPageDown | Motion::PageDown);
                if (down && line == last) || (!down && line == 0) {
                    return None;
                }
                let target = if down {
                    (line + lines).min(last)
                } else {
                    line.saturating_sub(lines)
                };
                vertical(target)
            }
            Motion::ScreenTop | Motion::ScreenMiddle | Motion::ScreenBottom => {
                let visible = host.visible_lines();
                let first = visible.start.min(last);
                let end = visible.end.clamp(first + 1, last + 1);
                let target = match motion {
                    Motion::ScreenTop => (first + n - 1).min(end - 1),
                    Motion::ScreenMiddle => first + (end - first - 1) / 2,
                    _ => end.saturating_sub(n).max(first),
                };
                Moved::linewise(first_non_blank(buffer, target))
            }
            Motion::Mark { name, linewise } => {
                let offset = self.mark(buffer, *name)?;
                if *linewise {
                    Moved::linewise(first_non_blank(buffer, buffer.line_of(offset)))
                } else {
                    Moved::exclusive(offset)
                }
            }
        })
    }

    fn set_search(&mut self, pattern: String, forward: bool) {
        self.registers.last_search = Some(pattern.clone());
        self.search = Some(LastSearch { pattern, forward });
    }

    fn find_pattern(
        &mut self,
        buffer: &TextBuffer,
        from: usize,
        pattern: &str,
        forward: bool,
        count: usize,
    ) -> Option<usize> {
        let regex =
            match search::compile(pattern, self.options.ignore_case, self.options.smart_case) {
                Ok(regex) => regex,
                Err(error) => {
                    self.show_message(error, true);
                    return None;
                }
            };
        self.highlight_search = true;
        match search::find(buffer.text(), &regex, from, forward, count) {
            Some(found) => {
                if found.wrapped {
                    self.show_message(
                        if forward {
                            "search hit BOTTOM, continuing at TOP"
                        } else {
                            "search hit TOP, continuing at BOTTOM"
                        },
                        false,
                    );
                }
                Some(found.start)
            }
            None => {
                self.show_message(format!("E486: Pattern not found: {pattern}"), true);
                None
            }
        }
    }

    // ---- Text objects ----------------------------------------------------------

    fn eval_object(
        &self,
        buffer: &TextBuffer,
        offset: usize,
        object: TextObject,
        count: Option<usize>,
    ) -> Option<Selected> {
        let n = count.unwrap_or(1).max(1);
        match object {
            TextObject::Word { around, big } => object::word(buffer, offset, n, around, big),
            TextObject::Quote { around, quote } => object::quote(buffer, offset, quote, around),
            TextObject::Bracket {
                around,
                open,
                close,
            } => object::bracket(buffer, offset, open, close, n, around),
            TextObject::Paragraph { around } => object::paragraph(buffer, offset, n, around),
        }
    }

    /// A text object typed in Visual mode selects it.
    fn select_object(
        &mut self,
        editor: &mut EditorMut<'_>,
        object: TextObject,
        count: Option<usize>,
    ) {
        let cursor = editor.cursor();
        let Some(selected) = self.eval_object(editor, cursor, object, count) else {
            self.failed = true;
            return;
        };
        if selected.range.is_empty() {
            return;
        }
        if selected.linewise {
            self.mode = Mode::Visual(VisualKind::Line);
        }
        let last = editor.prev_grapheme(selected.range.end);
        editor.set_selection(Selection::spanning(
            selected.range.start,
            last.max(selected.range.start),
        ));
    }

    // ---- Operators -------------------------------------------------------------

    fn operate(
        &mut self,
        editor: &mut EditorMut<'_>,
        host: &mut dyn Host,
        command: &Command,
        operator: Operator,
        target: &Target,
    ) {
        let selection = editor.selection_state();
        let cursor = selection.cursor;
        let visual = match self.mode {
            Mode::Visual(kind) => Some(kind),
            _ => None,
        };
        let region = match target {
            Target::Visual => {
                let Some(kind) = visual else {
                    return;
                };
                visual_region(editor, selection, kind)
            }
            Target::Lines => match visual {
                Some(kind) => {
                    let (first, last) = visual_region(editor, selection, kind).lines(editor);
                    Region::Lines(first, last)
                }
                None => {
                    let line = editor.line_of(cursor);
                    let last =
                        (line + command.count.unwrap_or(1).max(1) - 1).min(editor.line_count() - 1);
                    Region::Lines(line, last)
                }
            },
            Target::Motion(motion) => {
                let Some(moved) = self.eval_motion(
                    editor,
                    selection,
                    host,
                    motion,
                    command.count,
                    Some(operator),
                ) else {
                    self.failed = true;
                    return;
                };
                motion_region(editor, cursor, moved)
            }
            Target::Object(object) => {
                let Some(selected) = self.eval_object(editor, cursor, *object, command.count)
                else {
                    self.failed = true;
                    return;
                };
                if selected.linewise {
                    let last = editor.line_of(
                        selected
                            .range
                            .end
                            .saturating_sub(1)
                            .max(selected.range.start),
                    );
                    Region::Lines(editor.line_of(selected.range.start), last)
                } else {
                    Region::Chars(selected.range)
                }
            }
        };
        if let Some(kind) = visual {
            self.remember_visual(editor, kind);
            self.mode = Mode::Normal;
        }
        let shifts = if visual.is_some() {
            command.count.unwrap_or(1).max(1)
        } else {
            1
        };
        self.apply_operator(editor, host, command, operator, region, shifts);
    }

    fn apply_operator(
        &mut self,
        editor: &mut EditorMut<'_>,
        host: &mut dyn Host,
        command: &Command,
        operator: Operator,
        region: Region,
        shifts: usize,
    ) {
        let cursor = editor.cursor();
        match operator {
            Operator::Yank => {
                let register = region_register(editor, &region);
                self.store(command.register, register, Store::Yank, host);
                let offset = match region {
                    Region::Chars(ref range) => range.start,
                    Region::Lines(first, _) => {
                        if editor.line_of(cursor) == first {
                            cursor
                        } else {
                            motion::at_column(editor, first, editor.display_column(cursor))
                        }
                    }
                    Region::Block { first, left, .. } => {
                        editor.offset_at_display_column(first, left)
                    }
                };
                self.set_cursor(editor, offset);
            }
            Operator::Delete => {
                let register = region_register(editor, &region);
                self.store(command.register, register, Store::Delete, host);
                let offset = self.delete_region(editor, &region);
                self.set_cursor(editor, offset);
            }
            Operator::Change => {
                let register = region_register(editor, &region);
                self.store(command.register, register, Store::Delete, host);
                let (offset, block) = self.change_region(editor, &region);
                self.start_insert(editor, offset, Some(command.clone()), 1, None, false);
                if let Some(session) = self.insert.as_mut() {
                    session.block = block;
                }
            }
            Operator::Indent | Operator::Outdent => {
                let (first, last) = region.lines(editor);
                self.shift_lines(editor, first, last, operator == Operator::Indent, shifts);
                let offset = first_non_blank(editor, first);
                self.set_cursor(editor, offset);
            }
            Operator::Lowercase | Operator::Uppercase | Operator::ToggleCase => {
                let ranges = region_ranges(editor, &region);
                let start = ranges.first().map_or(cursor, |range| range.start);
                let edits = ranges
                    .into_iter()
                    .filter_map(|range| {
                        let text = &editor.text()[range.clone()];
                        let changed = change_case(text, operator);
                        (changed != text).then(|| Edit::replace(range, changed))
                    })
                    .collect();
                let _ = editor.edit(EditKind::Other, edits, |_, _| Selection::caret(start));
                let offset = match region {
                    Region::Lines(first, _) if editor.line_of(cursor) != first => {
                        first_non_blank(editor, first)
                    }
                    Region::Lines(..) => cursor,
                    _ => start,
                };
                self.set_cursor(editor, offset);
            }
        }
    }

    /// Delete a region and return where the cursor goes.
    fn delete_region(&mut self, editor: &mut EditorMut<'_>, region: &Region) -> usize {
        match *region {
            Region::Chars(ref range) => {
                let start = range.start;
                let _ = editor.edit(
                    EditKind::Other,
                    vec![Edit::delete(range.clone())],
                    |_, _| Selection::caret(start),
                );
                start
            }
            Region::Lines(first, last) => {
                let mut range = editor.line_start(first)..editor.line_range_with_break(last).end;
                let ends_without_break = range.end == editor.line_end(last);
                if ends_without_break && first > 0 {
                    range.start = editor.line_end(first - 1);
                }
                let _ = editor.edit(EditKind::Other, vec![Edit::delete(range)], |_, _| {
                    Selection::caret(0)
                });
                let line = first.min(editor.line_count() - 1);
                first_non_blank(editor, line)
            }
            Region::Block { .. } => {
                let ranges = region_ranges(editor, region);
                let Some(start) = ranges.first().map(|range| range.start) else {
                    return editor.cursor();
                };
                let edits = ranges.into_iter().map(Edit::delete).collect();
                let _ = editor.edit(EditKind::Other, edits, |_, _| Selection::caret(start));
                start
            }
        }
    }

    /// Delete a region for `c` and return where typing starts, with the block
    /// to replicate the typing onto.
    fn change_region(
        &mut self,
        editor: &mut EditorMut<'_>,
        region: &Region,
    ) -> (usize, Option<super::BlockInsert>) {
        match *region {
            Region::Chars(ref range) => {
                let start = range.start;
                let _ = editor.edit(
                    EditKind::Other,
                    vec![Edit::delete(range.clone())],
                    |_, _| Selection::caret(start),
                );
                (start, None)
            }
            Region::Lines(first, last) => {
                let indent = leading_whitespace(editor, first);
                let start = editor.line_start(first);
                let range = start..editor.line_end(last);
                let _ = editor.edit(
                    EditKind::Other,
                    vec![Edit::replace(range, indent.clone())],
                    |_, _| Selection::caret(start),
                );
                (start + indent.len(), None)
            }
            Region::Block {
                first,
                last,
                left,
                right,
            } => {
                let start = self.delete_region(editor, region);
                let offset = if editor.line_of(start) == first {
                    start
                } else {
                    editor.offset_at_display_column(first, left)
                };
                let block = (last > first).then_some(super::BlockInsert {
                    lines: (first + 1, last),
                    column: left,
                    to_line_end: right == usize::MAX,
                });
                (offset, block)
            }
        }
    }

    fn shift_lines(
        &mut self,
        editor: &mut EditorMut<'_>,
        first: usize,
        last: usize,
        indent: bool,
        times: usize,
    ) {
        let edits: Vec<Edit> = (first..=last)
            .filter_map(|line| {
                let start = editor.line_start(line);
                let content = editor.line(line);
                if indent {
                    return (!content.is_empty())
                        .then(|| Edit::insert(start, crate::editor::normal::INDENT.repeat(times)));
                }
                let mut remove = 0;
                for _ in 0..times {
                    let rest = &content[remove..];
                    remove += if rest.starts_with('\t') {
                        1
                    } else {
                        rest.chars().take(2).take_while(|c| *c == ' ').count()
                    };
                }
                (remove > 0).then(|| Edit::delete(start..start + remove))
            })
            .collect();
        let cursor = editor.cursor();
        let _ = editor.edit(EditKind::Other, edits, |changes, _| {
            Selection::caret(changes.map(cursor, Assoc::Before))
        });
    }

    pub(super) fn store(
        &mut self,
        name: Option<char>,
        register: Register,
        how: Store,
        host: &mut dyn Host,
    ) {
        if let Err(error) =
            self.registers
                .store(name, register, how, self.options.system_clipboard, host)
        {
            self.show_message(error, true);
        }
    }

    pub(super) fn read_register(
        &mut self,
        name: Option<char>,
        host: &mut dyn Host,
    ) -> Option<Register> {
        match self
            .registers
            .get(name, self.options.system_clipboard, host)
        {
            Ok(Some(register)) => Some(register),
            Ok(None) => {
                let name = name.unwrap_or('"');
                self.show_message(format!("E353: Nothing in register {name}"), true);
                None
            }
            Err(error) => {
                self.show_message(error, true);
                None
            }
        }
    }

    // ---- Single commands -------------------------------------------------------

    fn simple(
        &mut self,
        editor: &mut EditorMut<'_>,
        host: &mut dyn Host,
        command: &Command,
        simple: &Simple,
    ) {
        let count = command.count.unwrap_or(1).max(1);
        let operate = |operator, target| Command {
            register: command.register,
            count: command.count,
            action: Action::Operate(operator, target),
        };
        let visual = matches!(self.mode, Mode::Visual(_));
        match simple {
            Simple::Insert(kind) => self.insert_command(editor, command, *kind),
            Simple::Visual(kind) => self.toggle_visual(editor, *kind),
            Simple::DeleteChar { before } => {
                let motion = if *before { Motion::Left } else { Motion::Right };
                let delete = operate(Operator::Delete, Target::Motion(motion.clone()));
                self.operate(
                    editor,
                    host,
                    &delete,
                    Operator::Delete,
                    &Target::Motion(motion),
                );
            }
            Simple::Substitute => {
                let selection = editor.selection_state();
                let cursor = selection.cursor;
                let region = self
                    .eval_motion(
                        editor,
                        selection,
                        host,
                        &Motion::Right,
                        command.count,
                        Some(Operator::Change),
                    )
                    .map_or(Region::Chars(cursor..cursor), |moved| {
                        motion_region(editor, cursor, moved)
                    });
                self.apply_operator(editor, host, command, Operator::Change, region, 1);
            }
            Simple::SubstituteLine => self.operate(
                editor,
                host,
                &operate(Operator::Change, Target::Lines),
                Operator::Change,
                &Target::Lines,
            ),
            Simple::DeleteToEnd => {
                let target = Target::Motion(Motion::LineEnd);
                self.operate(
                    editor,
                    host,
                    &operate(Operator::Delete, target.clone()),
                    Operator::Delete,
                    &target,
                );
            }
            Simple::ChangeToEnd => {
                let target = Target::Motion(Motion::LineEnd);
                self.operate(
                    editor,
                    host,
                    &operate(Operator::Change, target.clone()),
                    Operator::Change,
                    &target,
                );
            }
            Simple::YankLine => self.operate(
                editor,
                host,
                &operate(Operator::Yank, Target::Lines),
                Operator::Yank,
                &Target::Lines,
            ),
            Simple::Join { spaces } => self.join(editor, count, command.count.is_some(), *spaces),
            Simple::ToggleCaseChar => self.toggle_case_chars(editor, count),
            Simple::Put { before } => {
                if visual {
                    self.visual_put(editor, host, command.register, count);
                } else {
                    self.put(editor, host, command.register, *before, count);
                }
            }
            Simple::Undo => self.undo(editor, count, false),
            Simple::Redo => self.undo(editor, count, true),
            Simple::Repeat => self.repeat(editor, host, command.count),
            Simple::ReplaceChar(replacement) => {
                if visual {
                    self.replace_visual(editor, *replacement);
                } else {
                    self.replace_chars(editor, count, *replacement);
                }
            }
            Simple::ReplaceMode => {
                let cursor = editor.cursor();
                self.start_insert(editor, cursor, Some(command.clone()), count, None, true);
            }
            Simple::CommandLine => {
                let visual_lines = match self.mode {
                    Mode::Visual(kind) => {
                        let lines =
                            visual_region(editor, editor.selection_state(), kind).lines(editor);
                        self.exit_visual(editor);
                        Some(lines)
                    }
                    _ => None,
                };
                let text = match (visual_lines, command.count) {
                    (Some(_), _) => "'<,'>".to_owned(),
                    (None, Some(1)) => ".".to_owned(),
                    (None, Some(count)) => format!(".,.+{}", count - 1),
                    (None, None) => String::new(),
                };
                self.command_line = Some(CommandLine {
                    prefix: ':',
                    text,
                    search: None,
                    visual_lines,
                });
            }
            Simple::Escape => {
                if visual {
                    self.exit_visual(editor);
                }
            }
            Simple::ScrollCursor(to) => host.request(Request::Scroll((*to).into())),
            Simple::SwapVisualEnds => {
                let selection = editor.selection_state();
                if let Some(anchor) = selection.anchor {
                    editor.set_selection(Selection::spanning(selection.cursor, anchor));
                }
            }
            Simple::ReselectVisual => self.reselect_visual(editor),
            Simple::VisualInsert { append } => self.visual_insert(editor, command, *append),
            Simple::SetMark(name) => self.set_mark(editor.cursor(), *name),
            Simple::RecordMacro(name) => {
                self.recording = Some((*name, Vec::new()));
            }
            Simple::StopRecording => self.stop_recording(),
            Simple::PlayMacro(name) => self.play_macro(editor, host, *name, count),
            Simple::PlayLastMacro => match self.last_macro {
                Some(name) => self.play_macro(editor, host, name, count),
                None => self.show_message("E748: No previously used register", true),
            },
            Simple::JumpOlder => self.jump(editor, -(count as isize)),
            Simple::JumpNewer => self.jump(editor, count as isize),
            Simple::Increment(delta) => self.increment(editor, delta * count as i64),
        }
    }

    fn insert_command(&mut self, editor: &mut EditorMut<'_>, command: &Command, kind: InsertKind) {
        let cursor = editor.cursor();
        let line = editor.line_of(cursor);
        let offset = match kind {
            InsertKind::Before => cursor,
            InsertKind::After => {
                if cursor < editor.line_end(line) {
                    editor.next_grapheme(cursor)
                } else {
                    cursor
                }
            }
            InsertKind::LineStart => first_non_blank(editor, line),
            InsertKind::LineEnd => editor.line_end(line),
            InsertKind::LineBelow => self.open_line(editor, line, true),
            InsertKind::LineAbove => self.open_line(editor, line, false),
        };
        let count = command.count.unwrap_or(1).max(1);
        self.start_insert(
            editor,
            offset,
            Some(command.clone()),
            count,
            Some(kind),
            false,
        );
    }

    /// Open a line below or above `line` with its indentation; returns where
    /// typing starts.
    pub(super) fn open_line(
        &mut self,
        editor: &mut EditorMut<'_>,
        line: usize,
        below: bool,
    ) -> usize {
        let indent = leading_whitespace(editor, line);
        let newline = editor.line_ending();
        let (at, text, caret) = if below {
            let at = editor.line_end(line);
            (
                at,
                format!("{newline}{indent}"),
                at + newline.len() + indent.len(),
            )
        } else {
            let at = editor.line_start(line);
            (at, format!("{indent}{newline}"), at + indent.len())
        };
        let _ = editor.edit(EditKind::Other, vec![Edit::insert(at, text)], |_, _| {
            Selection::caret(caret)
        });
        caret
    }

    pub(super) fn start_insert(
        &mut self,
        editor: &mut EditorMut<'_>,
        offset: usize,
        repeat: Option<Command>,
        count: usize,
        kind: Option<InsertKind>,
        replace: bool,
    ) {
        editor.set_selection(Selection::caret(offset));
        self.mode = if replace { Mode::Replace } else { Mode::Insert };
        self.replaced.clear();
        self.insert = Some(InsertSession {
            repeat,
            events: Vec::new(),
            count: count.max(1),
            kind,
            block: None,
            visual: None,
        });
    }

    fn toggle_visual(&mut self, editor: &mut EditorMut<'_>, kind: VisualKind) {
        match self.mode {
            Mode::Visual(current) if current == kind => self.exit_visual(editor),
            Mode::Visual(_) => self.mode = Mode::Visual(kind),
            _ => {
                let cursor = editor.cursor();
                self.mode = Mode::Visual(kind);
                editor.set_selection(Selection::spanning(cursor, cursor));
            }
        }
    }

    pub(super) fn exit_visual(&mut self, editor: &mut EditorMut<'_>) {
        if let Mode::Visual(kind) = self.mode {
            self.remember_visual(editor, kind);
        }
        self.mode = Mode::Normal;
        let cursor = editor.cursor();
        self.set_cursor(editor, cursor);
    }

    fn remember_visual(&mut self, editor: &EditorMut<'_>, kind: VisualKind) {
        let selection = editor.selection_state();
        if let Some((_, document)) = self.view {
            self.last_visual = Some(LastVisual {
                document,
                kind,
                anchor: selection.anchor.unwrap_or(selection.cursor),
                cursor: selection.cursor,
            });
        }
    }

    fn reselect_visual(&mut self, editor: &mut EditorMut<'_>) {
        let Some(last) = self
            .last_visual
            .filter(|last| Some(last.document) == self.view.map(|view| view.1))
        else {
            self.failed = true;
            return;
        };
        self.mode = Mode::Visual(last.kind);
        editor.set_selection(Selection::spanning(
            last.anchor.min(editor.len()),
            last.cursor.min(editor.len()),
        ));
    }

    /// The size of the current visual selection, for repeating with `.`.
    fn visual_extent(&self, editor: &EditorMut<'_>) -> Option<VisualExtent> {
        let Mode::Visual(kind) = self.mode else {
            return None;
        };
        let selection = editor.selection_state();
        let region = visual_region(editor, selection, kind);
        let (first, last) = region.lines(editor);
        let width = match region {
            Region::Chars(ref range) if first == last => {
                editor.text()[range.clone()].chars().count()
            }
            Region::Chars(ref range) => editor.display_column(range.end),
            Region::Block { left, right, .. } => right.saturating_sub(left),
            Region::Lines(..) => 0,
        };
        Some(VisualExtent {
            kind,
            lines: last - first + 1,
            width,
            to_line_end: selection.preferred_column == Some(usize::MAX),
        })
    }

    /// Select text of `extent`'s size at the cursor, for repeating a visual
    /// operator.
    pub(super) fn select_extent(&mut self, editor: &mut EditorMut<'_>, extent: VisualExtent) {
        let cursor = editor.cursor();
        let line = editor.line_of(cursor);
        let last_line = (line + extent.lines - 1).min(editor.line_count() - 1);
        let end = match extent.kind {
            VisualKind::Char if extent.lines == 1 => {
                let mut end = cursor;
                for _ in 1..extent.width {
                    end = editor.next_grapheme(end).min(last_char(editor, line));
                }
                end
            }
            VisualKind::Char => {
                motion::at_column(editor, last_line, extent.width.saturating_sub(1))
            }
            VisualKind::Line => editor.line_start(last_line),
            VisualKind::Block => {
                let column = editor.display_column(cursor) + extent.width.saturating_sub(1);
                motion::at_column(editor, last_line, column)
            }
        };
        self.mode = Mode::Visual(extent.kind);
        editor.set_selection(Selection {
            cursor: end,
            anchor: Some(cursor),
            preferred_column: extent.to_line_end.then_some(usize::MAX),
        });
    }

    // ---- Changes ---------------------------------------------------------------

    fn join(&mut self, editor: &mut EditorMut<'_>, count: usize, counted: bool, spaces: bool) {
        let (first, mut last) = match self.mode {
            Mode::Visual(kind) => {
                let lines = visual_region(editor, editor.selection_state(), kind).lines(editor);
                self.mode = Mode::Normal;
                lines
            }
            _ => {
                let line = editor.line_of(editor.cursor());
                (line, line + if counted { count.max(2) - 1 } else { 1 })
            }
        };
        if last == first {
            last += 1;
        }
        let last = last.min(editor.line_count() - 1);
        if last <= first {
            self.failed = true;
            return;
        }
        let mut edits = Vec::new();
        for line in first..last {
            let start = editor.line_end(line);
            let next = line + 1;
            let end = if spaces {
                first_non_blank(editor, next)
            } else {
                editor.line_start(next)
            };
            let current = editor.line(line);
            let following = &editor.text()[end..editor.line_end(next)];
            let separator = if !spaces
                || current.is_empty()
                || current.ends_with([' ', '\t'])
                || following.is_empty()
                || following.starts_with(')')
            {
                ""
            } else {
                " "
            };
            edits.push(Edit::replace(start..end, separator));
        }
        let join_point = edits.last().map_or(0, |edit| edit.range.start);
        let _ = editor.edit(EditKind::Other, edits, |changes, _| {
            Selection::caret(changes.map(join_point, Assoc::Before))
        });
    }

    fn toggle_case_chars(&mut self, editor: &mut EditorMut<'_>, count: usize) {
        let cursor = editor.cursor();
        let end_of_line = editor.line_end(editor.line_of(cursor));
        if cursor >= end_of_line {
            self.failed = true;
            return;
        }
        let mut end = cursor;
        for _ in 0..count {
            if end >= end_of_line {
                break;
            }
            end = editor.next_grapheme(end);
        }
        let changed = change_case(&editor.text()[cursor..end], Operator::ToggleCase);
        let _ = editor.edit(
            EditKind::Other,
            vec![Edit::replace(cursor..end, changed)],
            |changes, buffer| {
                let end = changes.new_extent().map_or(cursor, |range| range.end);
                Selection::caret(end.min(last_char(buffer, buffer.line_of(cursor))))
            },
        );
    }

    fn replace_chars(&mut self, editor: &mut EditorMut<'_>, count: usize, replacement: char) {
        let cursor = editor.cursor();
        let end_of_line = editor.line_end(editor.line_of(cursor));
        let mut end = cursor;
        for _ in 0..count {
            if end >= end_of_line {
                self.failed = true;
                return;
            }
            end = editor.next_grapheme(end);
        }
        if replacement == '\n' {
            let newline = editor.line_ending();
            let _ = editor.edit(
                EditKind::Other,
                vec![Edit::replace(cursor..end, newline)],
                |_, _| Selection::caret(cursor + newline.len()),
            );
            return;
        }
        let text = replacement.to_string().repeat(count);
        let _ = editor.edit(
            EditKind::Other,
            vec![Edit::replace(cursor..end, text.clone())],
            |_, _| Selection::caret(cursor + text.len() - replacement.len_utf8()),
        );
    }

    fn replace_visual(&mut self, editor: &mut EditorMut<'_>, replacement: char) {
        let Mode::Visual(kind) = self.mode else {
            return;
        };
        let region = visual_region(editor, editor.selection_state(), kind);
        self.remember_visual(editor, kind);
        self.mode = Mode::Normal;
        let ranges = region_ranges(editor, &region);
        let start = ranges.first().map_or(editor.cursor(), |range| range.start);
        let edits = ranges
            .into_iter()
            .map(|range| {
                let text: String = editor.text()[range.clone()]
                    .chars()
                    .map(|c| {
                        if c == '\n' || c == '\r' {
                            c
                        } else {
                            replacement
                        }
                    })
                    .collect();
                Edit::replace(range, text)
            })
            .collect();
        let _ = editor.edit(EditKind::Other, edits, |_, _| Selection::caret(start));
        self.set_cursor(editor, start);
    }

    fn put(
        &mut self,
        editor: &mut EditorMut<'_>,
        host: &mut dyn Host,
        name: Option<char>,
        before: bool,
        count: usize,
    ) {
        let Some(register) = self.read_register(name, host) else {
            self.failed = true;
            return;
        };
        let newline = editor.line_ending();
        let cursor = editor.cursor();
        let line = editor.line_of(cursor);
        match register.kind {
            RegisterKind::Char => {
                let text = register.text.repeat(count).replace('\n', newline);
                let at = if before || cursor >= editor.line_end(line) {
                    cursor
                } else {
                    editor.next_grapheme(cursor)
                };
                let multiline = text.contains('\n');
                let _ = editor.edit(
                    EditKind::Other,
                    vec![Edit::insert(at, text.clone())],
                    |_, buffer| {
                        Selection::caret(if multiline || text.is_empty() {
                            at
                        } else {
                            buffer.prev_grapheme(at + text.len())
                        })
                    },
                );
            }
            RegisterKind::Line => {
                let mut body = register.text.clone();
                if !body.ends_with('\n') {
                    body.push('\n');
                }
                let body = body.repeat(count).replace('\n', newline);
                let (at, text, target) = if before {
                    (editor.line_start(line), body, line)
                } else {
                    let end = editor.line_range_with_break(line).end;
                    if end == editor.line_end(line) {
                        let trimmed = body.strip_suffix(newline).unwrap_or(&body);
                        (end, format!("{newline}{trimmed}"), line + 1)
                    } else {
                        (end, body, line + 1)
                    }
                };
                let _ = editor.edit(
                    EditKind::Other,
                    vec![Edit::insert(at, text)],
                    |_, buffer| Selection::caret(first_non_blank(buffer, target)),
                );
            }
            RegisterKind::Block => self.put_block(editor, &register, before, count),
        }
    }

    fn put_block(
        &mut self,
        editor: &mut EditorMut<'_>,
        register: &Register,
        before: bool,
        count: usize,
    ) {
        let pieces: Vec<String> = register
            .text
            .split('\n')
            .map(|piece| piece.repeat(count))
            .collect();
        let width = pieces
            .iter()
            .map(|piece| display_width(piece))
            .max()
            .unwrap_or(0);
        let cursor = editor.cursor();
        let line = editor.line_of(cursor);
        let column = editor.display_column(cursor)
            + if before || cursor >= editor.line_end(line) {
                0
            } else {
                char_width(editor, cursor)
            };
        let newline = editor.line_ending();
        let last = editor.line_count() - 1;
        let mut edits: Vec<Edit> = Vec::new();
        let mut appended = String::new();
        for (index, piece) in pieces.iter().enumerate() {
            let target = line + index;
            if target > last {
                appended.push_str(newline);
                appended.push_str(&spaces(column));
                appended.push_str(piece);
                continue;
            }
            let line_width = editor.line_display_width(target);
            let edit = if line_width < column {
                Edit::insert(
                    editor.line_end(target),
                    format!("{}{piece}", spaces(column - line_width)),
                )
            } else {
                let at = editor.offset_at_display_column(target, column);
                let padding = if at < editor.line_end(target) {
                    spaces(width - display_width(piece))
                } else {
                    String::new()
                };
                Edit::insert(at, format!("{piece}{padding}"))
            };
            edits.push(edit);
        }
        if !appended.is_empty() {
            let end = editor.len();
            match edits.last_mut() {
                Some(edit) if edit.range.start == end => edit.text.push_str(&appended),
                _ => edits.push(Edit::insert(end, appended)),
            }
        }
        let first_at = edits.first().map_or(cursor, |edit| edit.range.start);
        let _ = editor.edit(EditKind::Other, edits, |changes, _| {
            Selection::caret(changes.map(first_at, Assoc::Before))
        });
    }

    /// `p` in Visual mode: replace the selection with a register; the replaced
    /// text goes to the unnamed register.
    fn visual_put(
        &mut self,
        editor: &mut EditorMut<'_>,
        host: &mut dyn Host,
        name: Option<char>,
        count: usize,
    ) {
        let Mode::Visual(kind) = self.mode else {
            return;
        };
        let Some(register) = self.read_register(name, host) else {
            self.failed = true;
            return;
        };
        let region = visual_region(editor, editor.selection_state(), kind);
        self.remember_visual(editor, kind);
        self.mode = Mode::Normal;
        let replaced = region_register(editor, &region);
        let newline = editor.line_ending();
        let body = register.text.repeat(count).replace('\n', newline);
        let edits = match (&region, register.kind) {
            (Region::Lines(first, last), _) => {
                let range = editor.line_start(*first)..editor.line_end(*last);
                let text = body.strip_suffix(newline).unwrap_or(&body).to_owned();
                vec![Edit::replace(range, text)]
            }
            (Region::Chars(range), RegisterKind::Line) => {
                let trimmed = body.strip_suffix(newline).unwrap_or(&body);
                vec![Edit::replace(
                    range.clone(),
                    format!("{newline}{trimmed}{newline}"),
                )]
            }
            _ => {
                let ranges = region_ranges(editor, &region);
                let Some(first) = ranges.first().cloned() else {
                    return;
                };
                let mut edits = vec![Edit::replace(first, body)];
                edits.extend(ranges.into_iter().skip(1).map(Edit::delete));
                edits
            }
        };
        let start = edits
            .first()
            .map_or(editor.cursor(), |edit| edit.range.start);
        let _ = editor.edit(EditKind::Other, edits, |_, _| Selection::caret(start));
        let line = editor.line_of(start);
        let offset = if register.kind == RegisterKind::Line {
            first_non_blank(
                editor,
                line + usize::from(matches!(region, Region::Chars(_))),
            )
        } else {
            start
        };
        self.set_cursor(editor, offset);
        self.store(None, replaced, Store::Delete, host);
    }

    fn undo(&mut self, editor: &mut EditorMut<'_>, count: usize, redo: bool) {
        let mut restored = None;
        for _ in 0..count {
            let range = if redo { editor.redo() } else { editor.undo() };
            match range {
                Some(range) => restored = Some(range),
                None => break,
            }
        }
        match restored {
            Some(range) => self.set_cursor(editor, range.start),
            None => {
                self.failed = true;
                self.show_message(
                    if redo {
                        "Already at newest change"
                    } else {
                        "Already at oldest change"
                    },
                    false,
                );
            }
        }
    }

    /// `.`: repeat the last change at the cursor, typed text included.
    fn repeat(&mut self, editor: &mut EditorMut<'_>, host: &mut dyn Host, count: Option<usize>) {
        let Some(change) = self.last_change.clone() else {
            self.failed = true;
            return;
        };
        let mut command = change.command.clone();
        if count.is_some() {
            command.count = count;
        }
        let opened = !self.group_open;
        if opened {
            editor.begin_group();
            self.group_open = true;
        }
        if let Some(extent) = change.visual {
            self.select_extent(editor, extent);
        }
        self.run(editor, host, command);
        if let Some(events) = change.insert
            && self.is_inserting()
        {
            for event in events {
                self.insert_event(editor, event);
            }
            self.finish_insert(editor, true);
        }
        if opened && self.group_open {
            editor.end_group();
            self.group_open = false;
        }
    }

    fn increment(&mut self, editor: &mut EditorMut<'_>, delta: i64) {
        let cursor = editor.cursor();
        let range = editor.line_range(editor.line_of(cursor));
        let text = &editor.text()[range.clone()];
        let from = cursor - range.start;
        // The number under or after the cursor, with a minus sign directly
        // before it.
        let bytes = text.as_bytes();
        let mut start = if bytes.get(from).is_some_and(u8::is_ascii_digit) {
            from
        } else {
            let Some(found) = text[from..].find(|c: char| c.is_ascii_digit()) else {
                self.failed = true;
                return;
            };
            from + found
        };
        while start > 0 && bytes[start - 1].is_ascii_digit() {
            start -= 1;
        }
        let mut end = start;
        while bytes.get(end).is_some_and(u8::is_ascii_digit) {
            end += 1;
        }
        if start > 0 && bytes[start - 1] == b'-' {
            start -= 1;
        }
        let Ok(value) = text[start..end].parse::<i64>() else {
            self.failed = true;
            return;
        };
        let replacement = value.saturating_add(delta).to_string();
        let range = range.start + start..range.start + end;
        let at = range.start;
        let _ = editor.edit(
            EditKind::Other,
            vec![Edit::replace(range, replacement.clone())],
            |_, _| Selection::caret(at + replacement.len() - 1),
        );
    }

    // ---- Marks, jumps and macros -------------------------------------------------

    fn set_mark(&mut self, offset: usize, name: char) {
        let Some((_, document)) = self.view else {
            return;
        };
        if name.is_ascii_lowercase() || name == '\'' || name == '`' {
            let name = if name == '`' { '\'' } else { name };
            self.marks.insert((document, name), offset);
        } else {
            self.show_message(
                "E191: Argument must be a letter or forward/backward quote",
                true,
            );
        }
    }

    fn mark(&mut self, buffer: &TextBuffer, name: char) -> Option<usize> {
        let (_, document) = self.view?;
        let name = if name == '`' { '\'' } else { name };
        match self.marks.get(&(document, name)) {
            Some(&offset) => Some(normal_position(buffer, offset.min(buffer.len()))),
            None => {
                self.show_message("E20: Mark not set", true);
                None
            }
        }
    }

    fn push_jump(&mut self, offset: usize) {
        let Some((_, document)) = self.view else {
            return;
        };
        self.marks.insert((document, '\''), offset);
        self.jumps.truncate(self.jump_index);
        if self.jumps.last() != Some(&(document, offset)) {
            self.jumps.push((document, offset));
        }
        if self.jumps.len() > 100 {
            self.jumps.remove(0);
        }
        self.jump_index = self.jumps.len();
    }

    fn jump(&mut self, editor: &mut EditorMut<'_>, steps: isize) {
        let Some((_, document)) = self.view else {
            return;
        };
        let cursor = editor.cursor();
        if steps < 0 && self.jump_index == self.jumps.len() {
            // Remember where `<C-o>` started so `<C-i>` can come back.
            self.jumps.push((document, cursor));
        }
        let positions: Vec<usize> = (0..self.jumps.len())
            .filter(|&index| self.jumps[index].0 == document)
            .collect();
        let current = positions
            .iter()
            .position(|&index| index >= self.jump_index.min(self.jumps.len() - 1))
            .unwrap_or(positions.len().saturating_sub(1));
        let target = current as isize + steps;
        if target < 0 || target as usize >= positions.len() {
            self.failed = true;
            return;
        }
        self.jump_index = positions[target as usize];
        let offset = self.jumps[self.jump_index].1.min(editor.len());
        self.set_cursor(editor, offset);
    }

    fn stop_recording(&mut self) {
        let Some((name, mut keys)) = self.recording.take() else {
            return;
        };
        // The `q` that stopped recording is not part of the macro.
        keys.pop();
        let text: String = keys.iter().map(Key::notation).collect();
        self.registers.set_recorded(name, text);
    }

    fn play_macro(
        &mut self,
        editor: &mut EditorMut<'_>,
        host: &mut dyn Host,
        name: char,
        count: usize,
    ) {
        let text = match name {
            '@' => self
                .last_macro
                .and_then(|name| self.registers.named_text(name)),
            name => self.registers.named_text(name).or_else(|| {
                self.registers
                    .get(Some(name), false, host)
                    .ok()
                    .flatten()
                    .map(|register| register.text)
            }),
        };
        let Some(text) = text else {
            self.show_message(format!("E353: Nothing in register {name}"), true);
            return;
        };
        if self.macro_depth >= MAX_MACRO_DEPTH {
            self.show_message("E169: Command too recursive", true);
            self.failed = true;
            return;
        }
        self.last_macro = Some(name);
        let keys = Key::parse(&text);
        let recording = self.recording.take();
        self.macro_depth += 1;
        'count: for _ in 0..count {
            for &key in &keys {
                self.failed = false;
                self.feed(editor, host, key);
                if self.failed {
                    break 'count;
                }
            }
        }
        self.macro_depth -= 1;
        self.recording = recording;
    }
}

/// The keyword under or after the cursor on its line, with its start.
fn word_under_cursor(buffer: &TextBuffer, cursor: usize) -> Option<(usize, String)> {
    let line = buffer.line_range(buffer.line_of(cursor));
    let mut start = cursor;
    while start < line.end && motion::class(buffer, start, false) != 2 {
        start = buffer.next_grapheme(start);
    }
    if start >= line.end {
        return None;
    }
    while start > line.start && motion::class(buffer, buffer.prev_grapheme(start), false) == 2 {
        start = buffer.prev_grapheme(start);
    }
    let mut end = start;
    while end < line.end && motion::class(buffer, end, false) == 2 {
        end = buffer.next_grapheme(end);
    }
    Some((start, buffer.text()[start..end].to_owned()))
}

fn change_case(text: &str, operator: Operator) -> String {
    match operator {
        Operator::Lowercase => text.to_lowercase(),
        Operator::Uppercase => text.to_uppercase(),
        _ => text
            .chars()
            .flat_map(|c| {
                if c.is_uppercase() {
                    c.to_lowercase().collect::<Vec<_>>()
                } else {
                    c.to_uppercase().collect::<Vec<_>>()
                }
            })
            .collect(),
    }
}
