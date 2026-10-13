//! Vim's command grammar: `["x][count]operator[count]motion`, motions, text
//! objects and single commands, parsed from the keys typed so far.

use super::VisualKind;
use super::key::Key;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operator {
    Delete,
    Change,
    Yank,
    Indent,
    Outdent,
    Lowercase,
    Uppercase,
    ToggleCase,
}

impl Operator {
    /// Whether applying the operator changes the text.
    pub fn changes_text(self) -> bool {
        self != Operator::Yank
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FindKind {
    /// `f`: onto the next occurrence.
    To,
    /// `F`: onto the previous occurrence.
    ToBack,
    /// `t`: just before the next occurrence.
    Till,
    /// `T`: just after the previous occurrence.
    TillBack,
}

impl FindKind {
    pub fn reversed(self) -> Self {
        match self {
            Self::To => Self::ToBack,
            Self::ToBack => Self::To,
            Self::Till => Self::TillBack,
            Self::TillBack => Self::Till,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Motion {
    Left,
    Right,
    Up,
    Down,
    /// `<BS>`: left, continuing at the end of the previous line.
    WrapLeft,
    /// `<Space>`: right, continuing at the start of the next line.
    WrapRight,
    WordStart {
        big: bool,
    },
    WordBack {
        big: bool,
    },
    WordEnd {
        big: bool,
    },
    WordEndBack {
        big: bool,
    },
    LineStart,
    FirstNonBlank,
    LineEnd,
    NextLineFirstNonBlank,
    PrevLineFirstNonBlank,
    /// `_`: first non-blank, `count - 1` lines down.
    CurrentLineFirstNonBlank,
    /// `gg`: first line, or line `count`.
    FirstLine,
    /// `G`: last line, or line `count`.
    LastLine,
    Find {
        kind: FindKind,
        target: char,
    },
    RepeatFind {
        reverse: bool,
    },
    MatchPair,
    ParagraphForward,
    ParagraphBackward,
    SearchNext {
        reverse: bool,
    },
    /// `*` and `#`: search for the word under the cursor.
    SearchWord {
        forward: bool,
    },
    Search {
        pattern: String,
        forward: bool,
    },
    HalfPageDown,
    HalfPageUp,
    PageDown,
    PageUp,
    ScreenTop,
    ScreenMiddle,
    ScreenBottom,
    DisplayDown,
    DisplayUp,
    Mark {
        name: char,
        linewise: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextObject {
    Word {
        around: bool,
        big: bool,
    },
    Quote {
        around: bool,
        quote: char,
    },
    Bracket {
        around: bool,
        open: char,
        close: char,
    },
    Paragraph {
        around: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InsertKind {
    /// `i`
    Before,
    /// `a`
    After,
    /// `I`
    LineStart,
    /// `A`
    LineEnd,
    /// `o`
    LineBelow,
    /// `O`
    LineAbove,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollTo {
    Center,
    Top,
    Bottom,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Simple {
    Insert(InsertKind),
    Visual(VisualKind),
    /// `x` (forward) and `X`.
    DeleteChar {
        before: bool,
    },
    Substitute,
    SubstituteLine,
    DeleteToEnd,
    ChangeToEnd,
    YankLine,
    Join {
        spaces: bool,
    },
    ToggleCaseChar,
    Put {
        before: bool,
    },
    Undo,
    Redo,
    Repeat,
    ReplaceChar(char),
    ReplaceMode,
    CommandLine,
    Escape,
    ScrollCursor(ScrollTo),
    SwapVisualEnds,
    ReselectVisual,
    /// `I` and `A` on a visual selection.
    VisualInsert {
        append: bool,
    },
    SetMark(char),
    RecordMacro(char),
    StopRecording,
    PlayMacro(char),
    PlayLastMacro,
    JumpOlder,
    JumpNewer,
    Increment(i64),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Motion(Motion),
    Object(TextObject),
    /// The operator doubled (`dd`, `cc`, `>>`): whole lines.
    Lines,
    /// The visual selection.
    Visual,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Move(Motion),
    /// A text object typed in visual mode extends the selection.
    Select(TextObject),
    Operate(Operator, Target),
    Simple(Simple),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Command {
    pub register: Option<char>,
    pub count: Option<usize>,
    pub action: Action,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Parsed {
    /// More keys are needed.
    Incomplete,
    /// The keys do not form a command; they are discarded.
    Invalid,
    Done(Command),
    /// `/` or `?`: read a pattern on the command line, then search (as the
    /// target of `operator`, if one is pending).
    Search {
        register: Option<char>,
        count: Option<usize>,
        operator: Option<Operator>,
        forward: bool,
    },
}

/// Where parsing stands for the keys typed so far.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Context {
    Normal,
    Visual,
}

pub fn is_register(name: char) -> bool {
    name.is_ascii_alphanumeric() || matches!(name, '"' | '+' | '*' | '-' | '_' | '.')
}

struct Cursor<'a> {
    keys: &'a [Key],
    index: usize,
}

/// A step that ran out of keys or met one it can not use.
enum Stop {
    Incomplete,
    Invalid,
}

type Step<T> = Result<T, Stop>;

impl Cursor<'_> {
    fn next(&mut self) -> Step<Key> {
        let key = *self.keys.get(self.index).ok_or(Stop::Incomplete)?;
        self.index += 1;
        Ok(key)
    }

    fn peek(&self) -> Option<Key> {
        self.keys.get(self.index).copied()
    }

    fn char_arg(&mut self) -> Step<char> {
        match self.next()? {
            Key::Char(character) => Ok(character),
            Key::Enter => Ok('\n'),
            _ => Err(Stop::Invalid),
        }
    }

    fn count(&mut self) -> Option<usize> {
        let mut count: Option<usize> = None;
        while let Some(Key::Char(digit @ '0'..='9')) = self.peek() {
            if digit == '0' && count.is_none() {
                break;
            }
            self.index += 1;
            let value = digit.to_digit(10).unwrap() as usize;
            count = Some(
                count
                    .unwrap_or(0)
                    .saturating_mul(10)
                    .saturating_add(value)
                    .min(99_999),
            );
        }
        count
    }

    fn register(&mut self) -> Step<Option<char>> {
        if self.peek() != Some(Key::Char('"')) {
            return Ok(None);
        }
        self.index += 1;
        let name = self.char_arg()?;
        if is_register(name) {
            Ok(Some(name))
        } else {
            Err(Stop::Invalid)
        }
    }
}

fn multiply(a: Option<usize>, b: Option<usize>) -> Option<usize> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.saturating_mul(b).min(99_999)),
        (a, b) => a.or(b),
    }
}

/// Parse the keys typed since the last completed command.
pub fn parse(keys: &[Key], context: Context, recording: bool) -> Parsed {
    let mut cursor = Cursor { keys, index: 0 };
    match parse_command(&mut cursor, context, recording) {
        Ok(parsed) => parsed,
        Err(Stop::Incomplete) => Parsed::Incomplete,
        Err(Stop::Invalid) => Parsed::Invalid,
    }
}

fn parse_command(cursor: &mut Cursor<'_>, context: Context, recording: bool) -> Step<Parsed> {
    let mut register = cursor.register()?;
    let mut count = cursor.count();
    if register.is_none() {
        register = cursor.register()?;
        count = multiply(count, cursor.count());
    }
    let done = |count: Option<usize>, action| {
        Ok(Parsed::Done(Command {
            register,
            count,
            action,
        }))
    };
    let key = cursor.next()?;
    if matches!(key, Key::Char('/' | '?')) {
        return Ok(Parsed::Search {
            register,
            count,
            operator: None,
            forward: key == Key::Char('/'),
        });
    }
    if let Some(operator) = operator(cursor, key)? {
        if context == Context::Visual {
            return done(count, Action::Operate(operator, Target::Visual));
        }
        let inner = cursor.count();
        count = multiply(count, inner);
        let next = cursor.next()?;
        if matches!(next, Key::Char('/' | '?')) {
            return Ok(Parsed::Search {
                register,
                count,
                operator: Some(operator),
                forward: next == Key::Char('/'),
            });
        }
        if doubles(operator, cursor, next)? {
            return done(count, Action::Operate(operator, Target::Lines));
        }
        if let Key::Char(prefix @ ('i' | 'a')) = next {
            let object = text_object(cursor, prefix == 'a')?;
            return done(count, Action::Operate(operator, Target::Object(object)));
        }
        let motion = motion(cursor, next)?.ok_or(Stop::Invalid)?;
        return done(count, Action::Operate(operator, Target::Motion(motion)));
    }
    if context == Context::Visual
        && let Key::Char(prefix @ ('i' | 'a')) = key
    {
        return done(count, Action::Select(text_object(cursor, prefix == 'a')?));
    }
    if let Some(motion) = motion(cursor, key)? {
        return done(count, Action::Move(motion));
    }
    let simple = match context {
        Context::Normal => normal_simple(cursor, key, recording)?,
        Context::Visual => visual_simple(cursor, key)?,
    };
    done(count, simple)
}

/// The operator `key` starts, reading a `g` prefix if needed.
fn operator(cursor: &mut Cursor<'_>, key: Key) -> Step<Option<Operator>> {
    Ok(Some(match key {
        Key::Char('d') => Operator::Delete,
        Key::Char('c') => Operator::Change,
        Key::Char('y') => Operator::Yank,
        Key::Char('>') => Operator::Indent,
        Key::Char('<') => Operator::Outdent,
        Key::Char('g') => match cursor.peek() {
            None => return Err(Stop::Incomplete),
            Some(Key::Char('u')) => {
                cursor.index += 1;
                Operator::Lowercase
            }
            Some(Key::Char('U')) => {
                cursor.index += 1;
                Operator::Uppercase
            }
            Some(Key::Char('~')) => {
                cursor.index += 1;
                Operator::ToggleCase
            }
            Some(_) => return Ok(None),
        },
        _ => return Ok(None),
    }))
}

/// Whether `next` repeats the operator (`dd`, `gUU`, `gUgU`).
fn doubles(operator: Operator, cursor: &mut Cursor<'_>, next: Key) -> Step<bool> {
    let own = match operator {
        Operator::Delete => 'd',
        Operator::Change => 'c',
        Operator::Yank => 'y',
        Operator::Indent => '>',
        Operator::Outdent => '<',
        Operator::Lowercase => 'u',
        Operator::Uppercase => 'U',
        Operator::ToggleCase => '~',
    };
    let case = matches!(
        operator,
        Operator::Lowercase | Operator::Uppercase | Operator::ToggleCase
    );
    if next == Key::Char(own) {
        return Ok(true);
    }
    if case && next == Key::Char('g') {
        if cursor.peek().is_none() {
            return Err(Stop::Incomplete);
        }
        if cursor.peek() == Some(Key::Char(own)) {
            cursor.index += 1;
            return Ok(true);
        }
        // `g` starts a motion such as `gg`; let the motion parser see it.
        cursor.index -= 0;
    }
    Ok(false)
}

fn text_object(cursor: &mut Cursor<'_>, around: bool) -> Step<TextObject> {
    Ok(match cursor.next()? {
        Key::Char('w') => TextObject::Word { around, big: false },
        Key::Char('W') => TextObject::Word { around, big: true },
        Key::Char('p') => TextObject::Paragraph { around },
        Key::Char(quote @ ('"' | '\'' | '`')) => TextObject::Quote { around, quote },
        Key::Char('(' | ')' | 'b') => TextObject::Bracket {
            around,
            open: '(',
            close: ')',
        },
        Key::Char('[' | ']') => TextObject::Bracket {
            around,
            open: '[',
            close: ']',
        },
        Key::Char('{' | '}' | 'B') => TextObject::Bracket {
            around,
            open: '{',
            close: '}',
        },
        Key::Char('<' | '>') => TextObject::Bracket {
            around,
            open: '<',
            close: '>',
        },
        _ => return Err(Stop::Invalid),
    })
}

/// The motion `key` starts, or `None` when `key` is not a motion.
fn motion(cursor: &mut Cursor<'_>, key: Key) -> Step<Option<Motion>> {
    Ok(Some(match key {
        Key::Char('h') | Key::Left => Motion::Left,
        Key::Char('l') | Key::Right => Motion::Right,
        Key::Char('j') | Key::Down | Key::Ctrl('n' | 'j') => Motion::Down,
        Key::Char('k') | Key::Up | Key::Ctrl('p') => Motion::Up,
        Key::Backspace | Key::Ctrl('h') => Motion::WrapLeft,
        Key::Char(' ') => Motion::WrapRight,
        Key::Char('w') => Motion::WordStart { big: false },
        Key::Char('W') => Motion::WordStart { big: true },
        Key::Char('b') => Motion::WordBack { big: false },
        Key::Char('B') => Motion::WordBack { big: true },
        Key::Char('e') => Motion::WordEnd { big: false },
        Key::Char('E') => Motion::WordEnd { big: true },
        Key::Char('0') | Key::Home => Motion::LineStart,
        Key::Char('^') => Motion::FirstNonBlank,
        Key::Char('$') | Key::End => Motion::LineEnd,
        Key::Char('+') | Key::Enter | Key::Ctrl('m') => Motion::NextLineFirstNonBlank,
        Key::Char('-') => Motion::PrevLineFirstNonBlank,
        Key::Char('_') => Motion::CurrentLineFirstNonBlank,
        Key::Char('G') => Motion::LastLine,
        Key::Char(find @ ('f' | 'F' | 't' | 'T')) => Motion::Find {
            kind: match find {
                'f' => FindKind::To,
                'F' => FindKind::ToBack,
                't' => FindKind::Till,
                _ => FindKind::TillBack,
            },
            target: cursor.char_arg()?,
        },
        Key::Char(';') => Motion::RepeatFind { reverse: false },
        Key::Char(',') => Motion::RepeatFind { reverse: true },
        Key::Char('%') => Motion::MatchPair,
        Key::Char('}') => Motion::ParagraphForward,
        Key::Char('{') => Motion::ParagraphBackward,
        Key::Char('n') => Motion::SearchNext { reverse: false },
        Key::Char('N') => Motion::SearchNext { reverse: true },
        Key::Char('*') => Motion::SearchWord { forward: true },
        Key::Char('#') => Motion::SearchWord { forward: false },
        Key::Ctrl('d') => Motion::HalfPageDown,
        Key::Ctrl('u') => Motion::HalfPageUp,
        Key::Ctrl('f') | Key::PageDown => Motion::PageDown,
        Key::Ctrl('b') | Key::PageUp => Motion::PageUp,
        Key::Char('H') => Motion::ScreenTop,
        Key::Char('M') => Motion::ScreenMiddle,
        Key::Char('L') => Motion::ScreenBottom,
        Key::Char(quote @ ('\'' | '`')) => Motion::Mark {
            name: cursor.char_arg()?,
            linewise: quote == '\'',
        },
        Key::Char('g') => {
            let Some(next) = cursor.peek() else {
                return Err(Stop::Incomplete);
            };
            let motion = match next {
                Key::Char('g') => Motion::FirstLine,
                Key::Char('e') => Motion::WordEndBack { big: false },
                Key::Char('E') => Motion::WordEndBack { big: true },
                Key::Char('j') | Key::Down => Motion::DisplayDown,
                Key::Char('k') | Key::Up => Motion::DisplayUp,
                _ => return Ok(None),
            };
            cursor.index += 1;
            motion
        }
        _ => return Ok(None),
    }))
}

fn normal_simple(cursor: &mut Cursor<'_>, key: Key, recording: bool) -> Step<Action> {
    let simple = match key {
        Key::Char('i') | Key::Insert => Simple::Insert(InsertKind::Before),
        Key::Char('a') => Simple::Insert(InsertKind::After),
        Key::Char('I') => Simple::Insert(InsertKind::LineStart),
        Key::Char('A') => Simple::Insert(InsertKind::LineEnd),
        Key::Char('o') => Simple::Insert(InsertKind::LineBelow),
        Key::Char('O') => Simple::Insert(InsertKind::LineAbove),
        Key::Char('v') => Simple::Visual(VisualKind::Char),
        Key::Char('V') => Simple::Visual(VisualKind::Line),
        Key::Ctrl('v' | 'q') => Simple::Visual(VisualKind::Block),
        Key::Char('x') | Key::Delete => Simple::DeleteChar { before: false },
        Key::Char('X') => Simple::DeleteChar { before: true },
        Key::Char('s') => Simple::Substitute,
        Key::Char('S') => Simple::SubstituteLine,
        Key::Char('D') => Simple::DeleteToEnd,
        Key::Char('C') => Simple::ChangeToEnd,
        Key::Char('Y') => Simple::YankLine,
        Key::Char('J') => Simple::Join { spaces: true },
        Key::Char('~') => Simple::ToggleCaseChar,
        Key::Char('p') => Simple::Put { before: false },
        Key::Char('P') => Simple::Put { before: true },
        Key::Char('u') => Simple::Undo,
        Key::Ctrl('r') => Simple::Redo,
        Key::Char('.') => Simple::Repeat,
        Key::Char('r') => Simple::ReplaceChar(cursor.char_arg()?),
        Key::Char('R') => Simple::ReplaceMode,
        Key::Char(':') => Simple::CommandLine,
        Key::Esc => Simple::Escape,
        Key::Char('m') => Simple::SetMark(cursor.char_arg()?),
        Key::Char('q') if recording => Simple::StopRecording,
        Key::Char('q') => {
            let name = cursor.char_arg()?;
            if !is_register(name) || name == '_' {
                return Err(Stop::Invalid);
            }
            Simple::RecordMacro(name)
        }
        Key::Char('@') => match cursor.char_arg()? {
            '@' => Simple::PlayLastMacro,
            name if is_register(name) => Simple::PlayMacro(name),
            _ => return Err(Stop::Invalid),
        },
        Key::Ctrl('o') => Simple::JumpOlder,
        Key::Ctrl('i') | Key::Tab => Simple::JumpNewer,
        Key::Ctrl('a') => Simple::Increment(1),
        Key::Ctrl('x') => Simple::Increment(-1),
        Key::Char('z') => match cursor.next()? {
            Key::Char('z' | '.') => Simple::ScrollCursor(ScrollTo::Center),
            Key::Char('t') | Key::Enter => Simple::ScrollCursor(ScrollTo::Top),
            Key::Char('b' | '-') => Simple::ScrollCursor(ScrollTo::Bottom),
            _ => return Err(Stop::Invalid),
        },
        Key::Char('g') => match cursor.next()? {
            Key::Char('J') => Simple::Join { spaces: false },
            Key::Char('v') => Simple::ReselectVisual,
            _ => return Err(Stop::Invalid),
        },
        _ => return Err(Stop::Invalid),
    };
    Ok(Action::Simple(simple))
}

fn visual_simple(cursor: &mut Cursor<'_>, key: Key) -> Step<Action> {
    let operate = |operator, linewise| {
        Ok(Action::Operate(
            operator,
            if linewise {
                Target::Lines
            } else {
                Target::Visual
            },
        ))
    };
    let simple = match key {
        Key::Char('x') | Key::Delete => return operate(Operator::Delete, false),
        Key::Char('s') => return operate(Operator::Change, false),
        Key::Char('X' | 'D') => return operate(Operator::Delete, true),
        Key::Char('Y') => return operate(Operator::Yank, true),
        Key::Char('C' | 'S' | 'R') => return operate(Operator::Change, true),
        Key::Char('~') => return operate(Operator::ToggleCase, false),
        Key::Char('u') => return operate(Operator::Lowercase, false),
        Key::Char('U') => return operate(Operator::Uppercase, false),
        Key::Char('v') => Simple::Visual(VisualKind::Char),
        Key::Char('V') => Simple::Visual(VisualKind::Line),
        Key::Ctrl('v' | 'q') => Simple::Visual(VisualKind::Block),
        Key::Char('o' | 'O') => Simple::SwapVisualEnds,
        Key::Char('J') => Simple::Join { spaces: true },
        Key::Char('p') => Simple::Put { before: false },
        Key::Char('P') => Simple::Put { before: true },
        Key::Char('r') => Simple::ReplaceChar(cursor.char_arg()?),
        Key::Char('I') => Simple::VisualInsert { append: false },
        Key::Char('A') => Simple::VisualInsert { append: true },
        Key::Char(':') => Simple::CommandLine,
        Key::Esc | Key::Ctrl('c') => Simple::Escape,
        Key::Char('z') => match cursor.next()? {
            Key::Char('z' | '.') => Simple::ScrollCursor(ScrollTo::Center),
            Key::Char('t') | Key::Enter => Simple::ScrollCursor(ScrollTo::Top),
            Key::Char('b' | '-') => Simple::ScrollCursor(ScrollTo::Bottom),
            _ => return Err(Stop::Invalid),
        },
        Key::Char('g') => match cursor.next()? {
            Key::Char('J') => Simple::Join { spaces: false },
            Key::Char('v') => Simple::ReselectVisual,
            _ => return Err(Stop::Invalid),
        },
        _ => return Err(Stop::Invalid),
    };
    Ok(Action::Simple(simple))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn normal(source: &str) -> Parsed {
        parse(&Key::parse(source), Context::Normal, false)
    }

    fn command(register: Option<char>, count: Option<usize>, action: Action) -> Parsed {
        Parsed::Done(Command {
            register,
            count,
            action,
        })
    }

    #[test]
    fn counts_registers_operators_and_motions_combine() {
        assert_eq!(
            normal("2d3w"),
            command(
                None,
                Some(6),
                Action::Operate(
                    Operator::Delete,
                    Target::Motion(Motion::WordStart { big: false })
                )
            )
        );
        assert_eq!(
            normal("\"a3yy"),
            command(
                Some('a'),
                Some(3),
                Action::Operate(Operator::Yank, Target::Lines)
            )
        );
        assert_eq!(
            normal("3\"ap"),
            command(
                Some('a'),
                Some(3),
                Action::Simple(Simple::Put { before: false })
            )
        );
        assert_eq!(
            normal("ci("),
            command(
                None,
                None,
                Action::Operate(
                    Operator::Change,
                    Target::Object(TextObject::Bracket {
                        around: false,
                        open: '(',
                        close: ')'
                    })
                )
            )
        );
        assert_eq!(
            normal("gUU"),
            command(
                None,
                None,
                Action::Operate(Operator::Uppercase, Target::Lines)
            )
        );
        assert_eq!(
            normal("gUgU"),
            command(
                None,
                None,
                Action::Operate(Operator::Uppercase, Target::Lines)
            )
        );
        assert_eq!(
            normal("dgg"),
            command(
                None,
                None,
                Action::Operate(Operator::Delete, Target::Motion(Motion::FirstLine))
            )
        );
        assert_eq!(
            normal("10G"),
            command(None, Some(10), Action::Move(Motion::LastLine))
        );
        assert_eq!(
            normal("0"),
            command(None, None, Action::Move(Motion::LineStart))
        );
        assert_eq!(
            normal("dfx"),
            command(
                None,
                None,
                Action::Operate(
                    Operator::Delete,
                    Target::Motion(Motion::Find {
                        kind: FindKind::To,
                        target: 'x'
                    })
                )
            )
        );
    }

    #[test]
    fn partial_and_invalid_input_are_told_apart() {
        for partial in [
            "", "2", "d", "d2", "\"", "\"a", "g", "dg", "f", "di", "z", "gU", "q",
        ] {
            assert_eq!(normal(partial), Parsed::Incomplete, "{partial:?}");
        }
        for invalid in ["dZ", "\"#", "di;", "zq", "f<Esc>", "gZ"] {
            assert_eq!(normal(invalid), Parsed::Invalid, "{invalid:?}");
        }
    }

    #[test]
    fn searches_open_the_command_line_with_any_pending_operator() {
        assert_eq!(
            normal("2d/"),
            Parsed::Search {
                register: None,
                count: Some(2),
                operator: Some(Operator::Delete),
                forward: true
            }
        );
        assert_eq!(
            normal("?"),
            Parsed::Search {
                register: None,
                count: None,
                operator: None,
                forward: false
            }
        );
    }

    #[test]
    fn visual_mode_applies_operators_to_the_selection() {
        let visual = |source: &str| parse(&Key::parse(source), Context::Visual, false);
        assert_eq!(
            visual("d"),
            command(
                None,
                None,
                Action::Operate(Operator::Delete, Target::Visual)
            )
        );
        assert_eq!(
            visual("Y"),
            command(None, None, Action::Operate(Operator::Yank, Target::Lines))
        );
        assert_eq!(
            visual("iw"),
            command(
                None,
                None,
                Action::Select(TextObject::Word {
                    around: false,
                    big: false
                })
            )
        );
        assert_eq!(
            visual("3j"),
            command(None, Some(3), Action::Move(Motion::Down))
        );
    }

    #[test]
    fn macros_depend_on_whether_one_is_recording() {
        assert_eq!(
            parse(&Key::parse("qa"), Context::Normal, false),
            command(None, None, Action::Simple(Simple::RecordMacro('a')))
        );
        assert_eq!(
            parse(&Key::parse("q"), Context::Normal, true),
            command(None, None, Action::Simple(Simple::StopRecording))
        );
    }
}
