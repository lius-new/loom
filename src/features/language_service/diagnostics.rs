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
            (a.range.start, a.range.end, a.severity).cmp(&(b.range.start, b.range.end, b.severity))
        });
        out
    }
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
