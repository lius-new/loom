//! Push diagnostics (`textDocument/publishDiagnostics`) as the editor needs them:
//! byte ranges in the document's own text, a severity and a message.
use super::capabilities::PositionEncoding;
use super::position::{LineIndex, Position};
use serde_json::Value;
use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Error,
    Warning,
    Information,
    Hint,
}

impl Severity {
    fn from_protocol(value: Option<i64>) -> Self {
        match value {
            Some(2) => Self::Warning,
            Some(3) => Self::Information,
            Some(4) => Self::Hint,
            // 1, or absent: the protocol leaves it to the client, and an error is
            // the safest thing to show.
            _ => Self::Error,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub range: Range<usize>,
    pub severity: Severity,
    pub message: String,
    pub source: Option<String>,
    pub code: Option<String>,
}

/// What one `publishDiagnostics` notification says, before positions are
/// resolved against a text.
#[derive(Clone, Debug, PartialEq)]
pub struct Published {
    pub uri: String,
    /// The document version the server computed them for, if it said.
    pub version: Option<i64>,
    items: Vec<Raw>,
}

#[derive(Clone, Debug, PartialEq)]
struct Raw {
    start: Position,
    end: Position,
    severity: Severity,
    message: String,
    source: Option<String>,
    code: Option<String>,
}

fn position(value: &Value) -> Option<Position> {
    Some(Position {
        line: u32::try_from(value.get("line")?.as_u64()?).ok()?,
        character: u32::try_from(value.get("character")?.as_u64()?).unwrap_or(u32::MAX),
    })
}

/// Reads the notification's parameters. A diagnostic that is malformed is left
/// out; the rest still count. `None` when there is no document to attach to.
pub fn parse(params: &Value) -> Option<Published> {
    let uri = params.get("uri")?.as_str()?.to_owned();
    let version = params.get("version").and_then(Value::as_i64);
    let items = params
        .get("diagnostics")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|item| {
                    let range = item.get("range")?;
                    Some(Raw {
                        start: position(range.get("start")?)?,
                        end: position(range.get("end")?)?,
                        severity: Severity::from_protocol(
                            item.get("severity").and_then(Value::as_i64),
                        ),
                        message: item.get("message")?.as_str()?.to_owned(),
                        source: item
                            .get("source")
                            .and_then(Value::as_str)
                            .map(str::to_owned),
                        code: match item.get("code") {
                            Some(Value::String(code)) => Some(code.clone()),
                            Some(Value::Number(code)) => Some(code.to_string()),
                            _ => None,
                        },
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Some(Published {
        uri,
        version,
        items,
    })
}

impl Published {
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// The diagnostics as byte ranges in `text`, ordered by position. A range
    /// with no width is widened to the character after it (or before it at the
    /// end of the text) so there is something to underline.
    pub fn resolve(&self, text: &str, encoding: PositionEncoding) -> Vec<Diagnostic> {
        let index = LineIndex::new(text);
        let mut out: Vec<Diagnostic> = self
            .items
            .iter()
            .map(|raw| {
                let mut start = index.offset(raw.start, encoding);
                let mut end = index.offset(raw.end, encoding).max(start);
                if start == end {
                    match text[end..]
                        .chars()
                        .next()
                        .filter(|c| !matches!(c, '\n' | '\r'))
                    {
                        Some(next) => end += next.len_utf8(),
                        None => {
                            if let Some(previous) = text[..start]
                                .chars()
                                .next_back()
                                .filter(|c| !matches!(c, '\n' | '\r'))
                            {
                                start -= previous.len_utf8();
                            }
                        }
                    }
                }
                Diagnostic {
                    range: start..end,
                    severity: raw.severity,
                    message: raw.message.clone(),
                    source: raw.source.clone(),
                    code: raw.code.clone(),
                }
            })
            .collect();
        out.sort_by(|a, b| {
            (a.range.start, a.range.end, a.severity, &a.message, &a.code).cmp(&(
                b.range.start,
                b.range.end,
                b.severity,
                &b.message,
                &b.code,
            ))
        });
        // Servers sometimes report the same problem twice (rust-analyzer does for
        // a syntax error seen by two of its checks); one is enough to show.
        out.dedup();
        out
    }
}

/// Diagnostics computed for `old` carried over to `new`, the way an editor keeps
/// the underlines of a server that has not caught up with the typing yet. The
/// two texts differ by one stretch (their common beginning and end are found), so
/// a diagnostic before it stays, one after it moves by the change in length, and
/// one that touches the changed stretch is dropped: the server will say more once
/// it has analysed the new text.
pub fn carry(old: &str, new: &str, diagnostics: Vec<Diagnostic>) -> Vec<Diagnostic> {
    let limit = old.len().min(new.len());
    let mut prefix = old
        .bytes()
        .zip(new.bytes())
        .take_while(|(a, b)| a == b)
        .count()
        .min(limit);
    while !old.is_char_boundary(prefix) || !new.is_char_boundary(prefix) {
        prefix -= 1;
    }
    let mut suffix = old
        .bytes()
        .rev()
        .zip(new.bytes().rev())
        .take_while(|(a, b)| a == b)
        .count()
        .min(limit - prefix);
    while !old.is_char_boundary(old.len() - suffix) || !new.is_char_boundary(new.len() - suffix) {
        suffix -= 1;
    }
    let old_end = old.len() - suffix;
    let delta = new.len() as isize - old.len() as isize;
    diagnostics
        .into_iter()
        .filter_map(|mut found| {
            if found.range.end <= prefix {
                Some(found)
            } else if found.range.start >= old_end {
                found.range = (found.range.start as isize + delta) as usize
                    ..(found.range.end as isize + delta) as usize;
                Some(found)
            } else {
                None
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::language_service::capabilities::PositionEncoding::{Utf8, Utf16};
    use serde_json::json;

    fn published(diagnostics: Value) -> Published {
        parse(&json!({ "uri": "file:///a.rs", "version": 3, "diagnostics": diagnostics })).unwrap()
    }

    fn at(line: u32, character: u32) -> Value {
        json!({ "line": line, "character": character })
    }

    #[test]
    fn diagnostics_resolve_to_byte_ranges_in_the_chosen_encoding() {
        let text = "let a = 1;\nlet 😀 = x;\n";
        let p = published(json!([{
            "range": { "start": at(1, 4), "end": at(1, 6) },
            "severity": 2, "message": "unused", "source": "rustc", "code": "unused_variables"
        }]));
        let found = p.resolve(text, Utf16);
        assert_eq!(&text[found[0].range.clone()], "😀");
        assert_eq!(found[0].severity, Severity::Warning);
        assert_eq!(found[0].code.as_deref(), Some("unused_variables"));
        assert_eq!(found[0].source.as_deref(), Some("rustc"));
        // The same range in UTF-8 columns covers the emoji's four bytes.
        let p = published(json!([{
            "range": { "start": at(1, 4), "end": at(1, 8) }, "message": "m"
        }]));
        assert_eq!(&text[p.resolve(text, Utf8)[0].range.clone()], "😀");
    }

    #[test]
    fn missing_severity_is_an_error_and_numeric_codes_are_kept() {
        let p = published(json!([{
            "range": { "start": at(0, 0), "end": at(0, 1) }, "message": "m", "code": 308
        }]));
        let found = p.resolve("x", Utf16);
        assert_eq!(found[0].severity, Severity::Error);
        assert_eq!(found[0].code.as_deref(), Some("308"));
    }

    #[test]
    fn empty_ranges_are_widened_to_one_character() {
        let p = published(json!([
            { "range": { "start": at(0, 1), "end": at(0, 1) }, "message": "mid" },
            { "range": { "start": at(0, 3), "end": at(0, 3) }, "message": "end of line" },
            { "range": { "start": at(1, 0), "end": at(1, 0) }, "message": "empty line" },
        ]));
        let text = "abc\n";
        let found = p.resolve(text, Utf16);
        assert_eq!(&text[found[0].range.clone()], "b");
        assert_eq!(
            &text[found[1].range.clone()],
            "c",
            "widened backwards at the end of a line"
        );
        assert_eq!(
            found[2].range,
            4..4,
            "nothing to underline on an empty last line"
        );
    }

    #[test]
    fn positions_past_the_text_are_clamped_and_results_are_ordered() {
        let p = published(json!([
            { "range": { "start": at(9, 0), "end": at(9, 5) }, "message": "late" },
            { "range": { "start": at(0, 0), "end": at(0, 2) }, "message": "early" },
        ]));
        let found = p.resolve("ab\n", Utf16);
        assert_eq!(found[0].message, "early");
        assert!(found[1].range.end <= 3);
    }

    fn found(start: usize, end: usize) -> Diagnostic {
        Diagnostic {
            range: start..end,
            severity: Severity::Error,
            message: "m".into(),
            source: None,
            code: None,
        }
    }

    #[test]
    fn diagnostics_follow_the_text_when_the_server_is_behind() {
        let old = "aaaa\nbbbb\ncccc\n";
        // A line is inserted between the first and the last.
        let new = "aaaa\nbbbb\nNEW LINE\ncccc\n";
        let carried = carry(old, new, vec![found(0, 4), found(10, 14)]);
        assert_eq!(
            carried.iter().map(|d| d.range.clone()).collect::<Vec<_>>(),
            [0..4, 19..23],
            "before stays, after moves by the length of the insertion"
        );
        assert_eq!(&new[19..23], "cccc");
    }

    #[test]
    fn a_diagnostic_on_the_edited_stretch_is_dropped() {
        let carried = carry(
            "let x = 1;",
            "let y = 1;",
            vec![found(4, 5), found(0, 3), found(8, 10)],
        );
        let ranges: Vec<_> = carried.iter().map(|d| d.range.clone()).collect();
        assert_eq!(ranges, [0..3, 8..10]);
    }

    #[test]
    fn identical_texts_and_multibyte_edits_are_handled() {
        assert_eq!(carry("same", "same", vec![found(1, 3)]), vec![found(1, 3)]);
        let carried = carry("é1é", "é22é", vec![found(0, 2), found(3, 5)]);
        assert_eq!(carried, vec![found(0, 2), found(4, 6)]);
        assert_eq!(carry("", "new", vec![found(0, 0)]), vec![found(0, 0)]);
    }

    #[test]
    fn identical_diagnostics_are_shown_once() {
        let same = json!({ "range": { "start": at(0, 0), "end": at(0, 1) }, "message": "twice" });
        let other = json!({ "range": { "start": at(0, 0), "end": at(0, 1) }, "message": "other" });
        let found = published(json!([same, other, same])).resolve("x", Utf16);
        let messages: Vec<&str> = found.iter().map(|d| d.message.as_str()).collect();
        assert_eq!(messages.len(), 2, "{messages:?}");
    }

    #[test]
    fn malformed_entries_are_skipped_and_the_rest_kept() {
        let p = published(json!([
            { "message": "no range" },
            { "range": { "start": at(0, 0), "end": at(0, 1) } },
            { "range": { "start": { "line": -1, "character": 0 }, "end": at(0, 1) }, "message": "negative" },
            { "range": { "start": at(0, 0), "end": at(0, 1) }, "message": "fine" },
        ]));
        assert_eq!(p.resolve("x", Utf16).len(), 1);
    }

    #[test]
    fn an_empty_publication_clears_and_a_missing_uri_is_refused() {
        assert!(published(json!([])).is_empty());
        assert_eq!(parse(&json!({ "diagnostics": [] })), None);
        let unversioned = parse(&json!({ "uri": "file:///a.rs", "diagnostics": [] })).unwrap();
        assert_eq!(unversioned.version, None);
    }
}
