//! The command line: `/` and `?` searches and a small set of Ex commands.

use crate::model::text::{Edit, EditKind, EditorMut, Selection, TextBuffer};

use super::command::{Action, Command, Motion, Target};
use super::motion::first_non_blank;
use super::search;
use super::{CommandLine, Host, Key, Mode, PendingSearch, Request, Vim};

/// An Ex line range, zero-based and inclusive.
type Lines = (usize, usize);

impl Vim {
    pub(super) fn command_line_key(
        &mut self,
        editor: &mut EditorMut<'_>,
        key: Key,
        host: &mut dyn Host,
    ) {
        let Some(line) = self.command_line.as_mut() else {
            return;
        };
        match key {
            Key::Char(character) => line.text.push(character),
            Key::Backspace | Key::Ctrl('h') => {
                if line.text.pop().is_none() {
                    self.command_line = None;
                }
            }
            Key::Ctrl('u') => line.text.clear(),
            Key::Ctrl('w') => {
                let trimmed = line.text.trim_end().len();
                let start = line.text[..trimmed]
                    .rfind(|c: char| !(c.is_alphanumeric() || c == '_'))
                    .map_or(0, |index| index + 1);
                line.text.truncate(if start == trimmed {
                    trimmed.saturating_sub(1)
                } else {
                    start
                });
            }
            Key::Esc | Key::Ctrl('c') => self.command_line = None,
            Key::Enter | Key::Ctrl('m' | 'j') => {
                if let Some(line) = self.command_line.take() {
                    self.execute_command_line(editor, host, line);
                }
            }
            _ => {}
        }
    }

    fn execute_command_line(
        &mut self,
        editor: &mut EditorMut<'_>,
        host: &mut dyn Host,
        line: CommandLine,
    ) {
        if line.prefix == ':' {
            self.registers.last_command = Some(line.text.clone());
            self.ex(editor, host, &line.text, line.visual_lines);
            return;
        }
        let forward = line.prefix == '/';
        let pattern = if line.text.is_empty() {
            match self.search.as_ref() {
                Some(search) => search.pattern.clone(),
                None => {
                    self.show_message("E35: No previous regular expression", true);
                    return;
                }
            }
        } else {
            line.text
        };
        self.registers.last_search = Some(pattern.clone());
        self.search = Some(super::LastSearch {
            pattern: pattern.clone(),
            forward,
        });
        let pending = line.search.unwrap_or(PendingSearch {
            register: None,
            count: None,
            operator: None,
            forward,
        });
        let motion = Motion::Search { pattern, forward };
        let action = match pending.operator {
            Some(operator) => Action::Operate(operator, Target::Motion(motion)),
            None => Action::Move(motion),
        };
        self.run(
            editor,
            host,
            Command {
                register: pending.register,
                count: pending.count,
                action,
            },
        );
    }

    /// Run an Ex command line.
    fn ex(
        &mut self,
        editor: &mut EditorMut<'_>,
        host: &mut dyn Host,
        text: &str,
        visual: Option<Lines>,
    ) {
        let text = text.trim().trim_start_matches(':').trim();
        if text.is_empty() {
            return;
        }
        let cursor_line = editor.line_of(editor.cursor());
        let (range, rest) = match parse_range(editor, cursor_line, text, visual) {
            Ok(parsed) => parsed,
            Err(error) => {
                self.show_message(error, true);
                return;
            }
        };
        let name_len = rest
            .find(|c: char| !c.is_ascii_alphabetic())
            .unwrap_or(rest.len());
        let name = &rest[..name_len];
        let after = &rest[name_len..];
        let bang = after.starts_with('!');
        let args = after.trim_start_matches('!').trim();
        let current = editor.line_of(editor.cursor());
        match name {
            "" if rest.starts_with(['s', '&']) => {}
            "" => {
                if let Some((_, last)) = range {
                    self.goto_line(editor, last);
                } else if !rest.is_empty() {
                    self.not_a_command(text);
                }
            }
            "w" | "write" => host.request(Request::Save),
            "wq" | "x" | "xit" | "exit" => host.request(Request::SaveAndQuit),
            "q" | "quit" | "clo" | "close" => host.request(Request::Quit { force: bang }),
            "noh" | "nohlsearch" => self.highlight_search = false,
            "s" | "substitute" => {
                let lines = range.unwrap_or((current, current));
                self.substitute(editor, lines, args);
            }
            "d" | "delete" => {
                let (first, last) = range.unwrap_or((current, current));
                editor.set_cursor(editor.line_start(first));
                self.run(
                    editor,
                    host,
                    Command {
                        register: args.chars().next(),
                        count: Some(last - first + 1),
                        action: Action::Operate(super::command::Operator::Delete, Target::Lines),
                    },
                );
            }
            "y" | "yank" => {
                let (first, last) = range.unwrap_or((current, current));
                let cursor = editor.cursor();
                editor.set_cursor(editor.line_start(first));
                self.run(
                    editor,
                    host,
                    Command {
                        register: args.chars().next(),
                        count: Some(last - first + 1),
                        action: Action::Operate(super::command::Operator::Yank, Target::Lines),
                    },
                );
                editor.set_selection(Selection::caret(cursor));
            }
            "set" | "se" => self.set_option(args),
            _ => self.not_a_command(text),
        }
    }

    fn not_a_command(&mut self, text: &str) {
        self.show_message(format!("E492: Not an editor command: {text}"), true);
    }

    fn goto_line(&mut self, editor: &mut EditorMut<'_>, line: usize) {
        let cursor = editor.cursor();
        let line = line.min(editor.line_count() - 1);
        self.push_jump_at(cursor);
        let offset = first_non_blank(editor, line);
        editor.set_selection(Selection::caret(offset));
        self.mode = Mode::Normal;
    }

    /// `:s/pattern/replacement/flags` on `lines`.
    fn substitute(&mut self, editor: &mut EditorMut<'_>, lines: Lines, args: &str) {
        let mut chars = args.chars();
        let delimiter = match chars.next() {
            Some(c) if !c.is_alphanumeric() && c != '\\' && c != '"' && c != '|' => c,
            None => '/',
            Some(_) => {
                self.show_message(
                    "E146: Regular expressions can't be delimited by letters",
                    true,
                );
                return;
            }
        };
        let parts = split_unescaped(chars.as_str(), delimiter);
        let pattern = parts.first().cloned().unwrap_or_default();
        let replacement = parts.get(1).cloned().unwrap_or_default();
        let flags = parts.get(2).cloned().unwrap_or_default();
        let pattern = if pattern.is_empty() {
            match self.search.as_ref() {
                Some(search) => search.pattern.clone(),
                None => {
                    self.show_message("E35: No previous regular expression", true);
                    return;
                }
            }
        } else {
            pattern
        };
        let ignore_case = if flags.contains('I') {
            false
        } else {
            flags.contains('i') || self.options.ignore_case
        };
        let regex = match search::compile(
            &pattern,
            ignore_case,
            self.options.smart_case && !flags.contains('i'),
        ) {
            Ok(regex) => regex,
            Err(error) => {
                self.show_message(error, true);
                return;
            }
        };
        self.registers.last_search = Some(pattern.clone());
        self.search = Some(super::LastSearch {
            pattern: pattern.clone(),
            forward: true,
        });
        let global = flags.contains('g');
        let newline = editor.line_ending();
        let mut edits = Vec::new();
        for line in lines.0..=lines.1.min(editor.line_count() - 1) {
            let range = editor.line_range(line);
            let text = &editor.text()[range.clone()];
            for captures in regex.captures_iter(text) {
                let whole = captures.get(0).expect("group 0 always matches");
                let expanded = expand(&replacement, &captures).replace('\n', newline);
                edits.push(Edit::replace(
                    range.start + whole.start()..range.start + whole.end(),
                    expanded,
                ));
                if !global {
                    break;
                }
            }
        }
        if edits.is_empty() {
            self.show_message(format!("E486: Pattern not found: {pattern}"), true);
            return;
        };
        let count = edits.len();
        let opened = !self.group_open;
        if opened {
            editor.begin_group();
        }
        let applied = editor.edit(EditKind::Other, edits, |changes, buffer| {
            // Line breaks inserted by `\r` move the last changed line down.
            let end = changes.new_extent().map_or(0, |range| range.end);
            Selection::caret(first_non_blank(buffer, buffer.line_of(end)))
        });
        if opened {
            editor.end_group();
        }
        if let Err(error) = applied {
            self.show_message(error.to_string(), true);
            return;
        }
        // Lines added by `\r` in the replacement move the last line down.
        let line = editor.line_of(editor.cursor());
        editor.set_selection(Selection::caret(first_non_blank(editor, line)));
        if count > 2 {
            self.show_message(
                format!("{count} substitutions on {} lines", lines.1 - lines.0 + 1),
                false,
            );
        }
    }

    fn set_option(&mut self, args: &str) {
        for option in args.split_whitespace() {
            let (name, value) = match option.strip_prefix("no") {
                Some(name) => (name, false),
                None => (option, true),
            };
            match name {
                "ignorecase" | "ic" => self.options.ignore_case = value,
                "smartcase" | "scs" => self.options.smart_case = value,
                "hlsearch" | "hls" => self.highlight_search = value,
                "clipboard" => {}
                _ => {
                    self.show_message(format!("E518: Unknown option: {option}"), true);
                    return;
                }
            }
        }
    }

    pub(super) fn push_jump_at(&mut self, offset: usize) {
        if let Some((_, document)) = self.view {
            self.marks.insert((document, '\''), offset);
            self.jumps.truncate(self.jump_index);
            self.jumps.push((document, offset));
            self.jump_index = self.jumps.len();
        }
    }
}

/// Parse a leading Ex range (`%`, `.`, `$`, numbers, `'<,'>`, `+n`, `-n`).
fn parse_range<'a>(
    buffer: &TextBuffer,
    cursor_line: usize,
    text: &'a str,
    visual: Option<Lines>,
) -> Result<(Option<Lines>, &'a str), String> {
    let last = buffer.line_count() - 1;
    if let Some(rest) = text.strip_prefix('%') {
        return Ok((Some((0, last)), rest.trim_start()));
    }
    let mut rest = text;
    let mut addresses = Vec::new();
    while let Some((address, after)) = parse_address(buffer, cursor_line, rest, visual)? {
        addresses.push(address.min(last));
        rest = after;
        match rest.strip_prefix(',').or_else(|| rest.strip_prefix(';')) {
            Some(after) => rest = after,
            None => break,
        }
    }
    let range = match addresses.as_slice() {
        [] => None,
        [line] => Some((*line, *line)),
        [first, .., second] => Some(((*first).min(*second), (*first).max(*second))),
    };
    Ok((range, rest.trim_start()))
}

/// One Ex address and the text after it.
fn parse_address<'a>(
    buffer: &TextBuffer,
    cursor_line: usize,
    text: &'a str,
    visual: Option<Lines>,
) -> Result<Option<(usize, &'a str)>, String> {
    let mut rest = text;
    let base: Option<isize> = if let Some(after) = rest.strip_prefix('.') {
        rest = after;
        None
    } else if let Some(after) = rest.strip_prefix('$') {
        rest = after;
        Some(buffer.line_count() as isize - 1)
    } else if let Some(after) = rest.strip_prefix("'<") {
        rest = after;
        Some(visual.ok_or("E20: Mark not set")?.0 as isize)
    } else if let Some(after) = rest.strip_prefix("'>") {
        rest = after;
        Some(visual.ok_or("E20: Mark not set")?.1 as isize)
    } else if rest.starts_with(|c: char| c.is_ascii_digit()) {
        let digits = rest
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(rest.len());
        let number: isize = rest[..digits].parse().map_err(|_| "E16: Invalid range")?;
        rest = &rest[digits..];
        Some((number - 1).max(0))
    } else if rest.starts_with(['+', '-']) {
        None
    } else {
        return Ok(None);
    };
    // `.` and a bare offset are relative to the cursor line.
    let mut line = base.unwrap_or(cursor_line as isize);
    while let Some(sign) = rest.chars().next().filter(|c| *c == '+' || *c == '-') {
        rest = &rest[1..];
        let digits = rest
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(rest.len());
        let amount: isize = if digits == 0 {
            1
        } else {
            rest[..digits].parse().unwrap_or(1)
        };
        rest = &rest[digits..];
        line += if sign == '+' { amount } else { -amount };
    }
    Ok(Some((line.max(0) as usize, rest)))
}

/// Split `text` at unescaped `delimiter`s, keeping other escapes.
fn split_unescaped(text: &str, delimiter: char) -> Vec<String> {
    let mut parts = vec![String::new()];
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some(next) if next == delimiter => parts.last_mut().unwrap().push(next),
                Some(next) => {
                    parts.last_mut().unwrap().push('\\');
                    parts.last_mut().unwrap().push(next);
                }
                None => parts.last_mut().unwrap().push('\\'),
            }
        } else if c == delimiter {
            parts.push(String::new());
        } else {
            parts.last_mut().unwrap().push(c);
        }
    }
    parts
}

/// Expand a Vim replacement string: `&` and `\0` are the match, `\1`-`\9`
/// groups, `\r` and `\n` line breaks, `\&` a literal ampersand.
fn expand(replacement: &str, captures: &regex::Captures<'_>) -> String {
    let mut out = String::new();
    let mut chars = replacement.chars();
    while let Some(c) = chars.next() {
        match c {
            '&' => out.push_str(captures.get(0).map_or("", |found| found.as_str())),
            '\\' => match chars.next() {
                Some(digit @ '0'..='9') => {
                    let group = digit.to_digit(10).unwrap() as usize;
                    out.push_str(captures.get(group).map_or("", |found| found.as_str()));
                }
                Some('r' | 'n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some(other) => out.push(other),
                None => out.push('\\'),
            },
            other => out.push(other),
        }
    }
    out
}
