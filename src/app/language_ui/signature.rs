//! Signature help: the parameters of the call the caret is in, shown above it.
use super::{AppContext, AppState, services, target};
use crate::app::ui_timer;
use crate::domain::identity::{FileId, PaneId};
use crate::features::language_service::reply::{self, SignatureHelp};
use crate::features::language_service::request::Feature;
use crate::features::markdown::{self, Block};
use crate::infrastructure::lsp::manager::{RequestHandle, RequestSpec};
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;

const DEBOUNCE: Duration = Duration::from_millis(20);

#[derive(Clone)]
pub struct Session {
    pub pane: PaneId,
    pub document: FileId,
    pub incarnation: u64,
    /// Where the call's opening bracket is, as far as is known.
    pub opened_at: usize,
    handle: Option<Arc<RequestHandle>>,
    pub help: Option<SignatureHelp>,
    last_cursor: usize,
    last_version: u64,
}

/// What to draw.
#[derive(Clone)]
pub struct View {
    pub anchor: usize,
    pub label: String,
    /// The active parameter, as character offsets into the label.
    pub active: Option<std::ops::Range<usize>>,
    pub docs: Option<Arc<[Block]>>,
    /// Which signature of how many, when there are several.
    pub position: Option<(usize, usize)>,
}

pub fn view(session: &Session) -> Option<View> {
    let help = session.help.as_ref()?;
    let signature = help.signatures.get(help.active_signature)?;
    let active = signature
        .active_parameter
        .or(help.active_parameter)
        .and_then(|index| signature.parameters.get(index))
        .map(|parameter| parameter.label.clone());
    let mut source = String::new();
    if let Some(parameter) = signature
        .active_parameter
        .or(help.active_parameter)
        .and_then(|index| signature.parameters.get(index))
        .and_then(|p| p.documentation.as_ref())
    {
        source.push_str(parameter);
        source.push_str("\n\n");
    }
    if let Some(documentation) = &signature.documentation {
        source.push_str(documentation);
    }
    Some(View {
        anchor: session.opened_at,
        label: signature.label.clone(),
        active,
        docs: (!source.trim().is_empty()).then(|| markdown::parse(&source).into()),
        position: (help.signatures.len() > 1)
            .then_some((help.active_signature + 1, help.signatures.len())),
    })
}

/// Which brackets trigger signature help, from the servers that offer it.
fn triggers(ctx: &AppContext) -> (Vec<String>, Vec<String>) {
    let mut open = Vec::new();
    let mut again = Vec::new();
    if let Some(services) = services(ctx) {
        for info in services.infos() {
            if let Some(options) = info
                .providers
                .as_ref()
                .and_then(|p| p.signature_help.as_ref())
            {
                open.extend(options.trigger_characters.clone());
                again.extend(options.retrigger_characters.clone());
            }
        }
    }
    (open, again)
}

/// Watches typing for the characters that open or refresh the help, and ends it
/// when the caret leaves the call.
pub fn observe(ctx: &AppContext, app: &mut AppState, typed: Option<char>) {
    let pane = app.workspace.active_pane();
    let Some(t) = target(app, pane) else {
        app.editor.language.signature = None;
        return;
    };
    let mut keep = true;
    if let Some(session) = app.editor.language.signature.as_mut() {
        if session.document != t.document || session.incarnation != t.stamp.incarnation {
            keep = false;
        } else if session.last_version != t.stamp.version || session.last_cursor != t.cursor {
            session.last_version = t.stamp.version;
            session.last_cursor = t.cursor;
            keep = inside_call(&t.text, session.opened_at, t.cursor);
        }
    }
    if !keep {
        app.editor.language.signature = None;
    }
    let Some(c) = typed else { return };
    let (open, again) = triggers(ctx);
    let opens = open.iter().any(|s| s.chars().eq([c]));
    let refresh =
        app.editor.language.signature.is_some() && again.iter().any(|s| s.chars().eq([c]));
    if opens || refresh {
        schedule(
            ctx,
            app,
            if opens {
                Some(t.cursor.saturating_sub(c.len_utf8()))
            } else {
                None
            },
        );
    }
}

/// Whether `cursor` is still inside the call opened at `opened_at`.
pub fn inside_call(text: &str, opened_at: usize, cursor: usize) -> bool {
    if cursor <= opened_at
        || cursor > text.len()
        || opened_at >= text.len()
        || !text.is_char_boundary(opened_at)
        || !text.is_char_boundary(cursor)
    {
        return false;
    }
    let (open, close) = match text[opened_at..].chars().next() {
        Some('(') => ('(', ')'),
        Some('<') => ('<', '>'),
        Some('[') => ('[', ']'),
        _ => return true,
    };
    let mut depth = 0i32;
    for c in text[opened_at..cursor].chars() {
        if c == open {
            depth += 1;
        } else if c == close {
            depth -= 1;
            if depth <= 0 {
                return false;
            }
        }
    }
    true
}

fn schedule(ctx: &AppContext, app: &mut AppState, opened_at: Option<usize>) {
    let Some(application) = ctx.services().and_then(|r| r.application()) else {
        return;
    };
    let timed = ctx.clone();
    app.editor.language.signature_timer = Some(Arc::new(ui_timer::after(
        application,
        DEBOUNCE,
        move || timed.update(|app| request(&timed, app, opened_at, false)),
    )));
}

/// Asks the servers now.
pub fn request(ctx: &AppContext, app: &mut AppState, opened_at: Option<usize>, manual: bool) {
    app.editor.language.signature_timer = None;
    let pane = app.workspace.active_pane();
    let Some(t) = target(app, pane) else { return };
    let Some(services) = services(ctx) else {
        return;
    };
    services.sync(app);
    let handle = Arc::new(
        services.request(
            RequestSpec::new(
                Feature::SignatureHelp,
                t.path.clone(),
                t.lsp_stamp(),
                t.cursor,
            )
            .extra(json!({ "context": {
                    "triggerKind": if manual { 1 } else { 2 },
                    "isRetrigger": app.editor.language.signature.is_some(),
                } })),
        ),
    );
    match app.editor.language.signature.as_mut() {
        Some(session) => session.handle = Some(handle),
        None => {
            app.editor.language.signature = Some(Session {
                pane,
                document: t.document,
                incarnation: t.stamp.incarnation,
                opened_at: opened_at.unwrap_or_else(|| open_bracket_before(&t.text, t.cursor)),
                handle: Some(handle),
                help: None,
                last_cursor: t.cursor,
                last_version: t.stamp.version,
            })
        }
    }
}

/// The innermost unclosed bracket before `cursor`, or `cursor` itself.
pub fn open_bracket_before(text: &str, cursor: usize) -> usize {
    let mut depth = 0i32;
    for (at, c) in text[..cursor.min(text.len())].char_indices().rev() {
        match c {
            ')' => depth += 1,
            '(' => {
                if depth == 0 {
                    return at;
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    cursor
}

/// Folds in the servers' answer.
pub fn tick(app: &mut AppState) -> bool {
    let Some(session) = app.editor.language.signature.as_mut() else {
        return false;
    };
    let Some(handle) = session.handle.clone() else {
        return false;
    };
    let progress = handle.poll();
    if !progress.finished {
        return false;
    }
    session.handle = None;
    let help = progress
        .usable(Feature::SignatureHelp)
        .first()
        .and_then(|part| reply::signature_help(&part.value));
    match help {
        Some(help) => session.help = Some(help),
        None => app.editor.language.signature = None,
    }
    true
}

pub fn close(app: &mut AppState) {
    app.editor.language.signature = None;
    app.editor.language.signature_timer = None;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_call_ends_at_its_closing_bracket() {
        let text = "f(a, g(b), c) + 1";
        assert!(inside_call(text, 1, 5));
        assert!(inside_call(text, 1, 11), "after a nested call closed");
        assert!(!inside_call(text, 1, 13), "after the closing bracket");
        assert!(!inside_call(text, 1, 1), "before anything was typed");
        assert!(inside_call(text, 6, 8), "the nested call itself");
    }

    #[test]
    fn stale_positions_end_the_call_instead_of_panicking() {
        // The text changed under the remembered position.
        assert!(!inside_call("é(a", 1, 3), "inside a character");
        assert!(!inside_call("(a", 5, 7));
        assert!(!inside_call("(a", 0, 9));
        assert!(!inside_call("é", 0, 1));
    }

    #[test]
    fn the_enclosing_open_bracket_skips_closed_calls() {
        let text = "f(a, g(b), ";
        assert_eq!(open_bracket_before(text, text.len()), 1);
        assert_eq!(open_bracket_before("f(a, g(b", 8), 6);
        assert_eq!(open_bracket_before("no brackets", 5), 5);
    }
}
