//! Semantic tokens: decoding the server's relative integer encoding into spans
//! the editor can draw, and mixing them over syntax-tree spans.
use super::capabilities::{PositionEncoding, SemanticLegend};
use super::position::{LineIndex, Position};
use crate::features::highlighting::HighlightId;
use crate::features::syntax::TokenSpan;
use serde_json::Value;
use std::ops::Range;

/// Why a response cannot be used. The previous result then stays unpublished.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Malformed(pub String);

/// The class a server's token type and modifiers are drawn with, if any.
pub fn class_for(token_type: &str, modifiers: &[&str]) -> Option<HighlightId> {
    let has = |name: &str| modifiers.contains(&name);
    Some(match token_type {
        "namespace" => HighlightId::MODULE,
        "type" | "class" | "enum" | "interface" | "struct" | "typeParameter" => {
            if has("defaultLibrary") {
                HighlightId::TYPE_BUILTIN
            } else {
                HighlightId::TYPE
            }
        }
        "parameter" => HighlightId::VARIABLE_PARAMETER,
        "variable" => {
            if has("readonly") && has("static") {
                HighlightId::CONSTANT
            } else {
                HighlightId::VARIABLE
            }
        }
        "property" => HighlightId::PROPERTY,
        "enumMember" => HighlightId::CONSTANT,
        "function" => {
            if has("defaultLibrary") {
                HighlightId::FUNCTION_BUILTIN
            } else {
                HighlightId::FUNCTION
            }
        }
        "method" => HighlightId::FUNCTION_METHOD,
        "macro" => HighlightId::FUNCTION_MACRO,
        "keyword" | "modifier" => HighlightId::KEYWORD,
        "comment" => HighlightId::COMMENT,
        "string" => HighlightId::STRING,
        "number" => HighlightId::NUMBER,
        "regexp" => HighlightId::STRING_REGEX,
        "operator" => HighlightId::OPERATOR,
        "decorator" => HighlightId::ATTRIBUTE,
        _ => return None,
    })
}

/// One edit of a delta response: it replaces `delete_count` integers of the
/// previous `data` array, starting at integer index `start`, by `data`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeltaEdit {
    pub start: usize,
    pub delete_count: usize,
    pub data: Vec<u32>,
}

/// The integers of a `data` array, or `None` if it is not an array of
/// non-negative integers that fit.
pub fn integers(value: &Value) -> Option<Vec<u32>> {
    value
        .as_array()?
        .iter()
        .map(|n| n.as_u64().and_then(|n| u32::try_from(n).ok()))
        .collect()
}

/// The edits of a `semanticTokens/full/delta` response, if it is one.
pub fn parse_delta(value: &Value) -> Option<Vec<DeltaEdit>> {
    value
        .get("edits")?
        .as_array()?
        .iter()
        .map(|edit| {
            Some(DeltaEdit {
                start: usize::try_from(edit.get("start")?.as_u64()?).ok()?,
                delete_count: usize::try_from(edit.get("deleteCount")?.as_u64()?).ok()?,
                data: match edit.get("data") {
                    Some(data) => integers(data)?,
                    None => Vec::new(),
                },
            })
        })
        .collect()
}

/// The new `data` array after applying `edits` to `old`. All edits describe the
/// old array, so they are checked to be in range and not to overlap, then applied
/// from the last to the first. An answer that breaks these rules, or leaves a length
/// that is not a whole number of tokens, is refused and the caller asks for a
/// full result instead.
pub fn apply_delta(old: &[u32], edits: &[DeltaEdit]) -> Result<Vec<u32>, Malformed> {
    let mut order: Vec<&DeltaEdit> = edits.iter().collect();
    order.sort_by_key(|edit| edit.start);
    let mut end_of_previous = 0;
    for edit in &order {
        let end = edit
            .start
            .checked_add(edit.delete_count)
            .filter(|end| *end <= old.len())
            .ok_or_else(|| Malformed("a delta edit reaches past the previous tokens".into()))?;
        if edit.start < end_of_previous {
            return Err(Malformed("delta edits overlap".into()));
        }
        end_of_previous = end;
    }
    let mut data = old.to_vec();
    for edit in order.iter().rev() {
        data.splice(
            edit.start..edit.start + edit.delete_count,
            edit.data.iter().copied(),
        );
    }
    if !data.len().is_multiple_of(5) {
        return Err(Malformed("the delta leaves a partial token".into()));
    }
    Ok(data)
}

/// Decodes a `data` array against the text it was computed for. Tokens whose type
/// the legend names but the editor does not draw are skipped. A token that
/// reaches past its line is cut at the line end, and one that overlaps the
/// previous token is dropped. The result is sorted and does not overlap.
pub fn decode(
    data: &Value,
    legend: &SemanticLegend,
    text: &str,
    encoding: PositionEncoding,
) -> Result<Vec<TokenSpan>, Malformed> {
    let Some(data) = data.as_array() else {
        return Err(Malformed("the token data is not an array".into()));
    };
    if data.len() % 5 != 0 {
        return Err(Malformed(format!(
            "the token data has {} numbers, which is not a multiple of five",
            data.len()
        )));
    }
    let number = |value: &Value| {
        value
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .ok_or_else(|| Malformed(format!("{value} is not a valid token number")))
    };
    let index = LineIndex::new(text);
    let mut out: Vec<TokenSpan> = Vec::new();
    let (mut line, mut start) = (0u32, 0u32);
    for token in data.chunks(5) {
        let (delta_line, delta_start, length, kind, modifiers) = (
            number(&token[0])?,
            number(&token[1])?,
            number(&token[2])?,
            number(&token[3])? as usize,
            number(&token[4])?,
        );
        line = line
            .checked_add(delta_line)
            .ok_or_else(|| Malformed("a token line overflows".into()))?;
        start = if delta_line == 0 {
            start
                .checked_add(delta_start)
                .ok_or_else(|| Malformed("a token column overflows".into()))?
        } else {
            delta_start
        };
        let Some(kind) = legend.token_types.get(kind) else {
            return Err(Malformed(format!("token type {kind} is not in the legend")));
        };
        let names: Vec<&str> = legend
            .token_modifiers
            .iter()
            .enumerate()
            .filter(|(bit, _)| *bit < 32 && modifiers & (1 << bit) != 0)
            .map(|(_, name)| name.as_str())
            .collect();
        let Some(class) = class_for(kind, &names) else {
            continue;
        };
        if length == 0 || line as usize >= index.line_count() {
            continue;
        }
        let from = index.offset(
            Position {
                line,
                character: start,
            },
            encoding,
        );
        let line_end = from.max(index.offset(
            Position {
                line,
                character: u32::MAX,
            },
            encoding,
        ));
        let to = index
            .offset(
                Position {
                    line,
                    character: start.saturating_add(length),
                },
                encoding,
            )
            .min(line_end);
        if to <= from || out.last().is_some_and(|previous| from < previous.range.end) {
            continue;
        }
        out.push(TokenSpan {
            range: from..to,
            class: Some(class),
        });
    }
    Ok(out)
}

/// `base`, which tiles `window`, with the classes of `overlay` (sorted, not
/// overlapping, classed) drawn over it. The result tiles `window` too, and
/// neighbouring pieces with the same class are joined.
pub fn compose(base: &[TokenSpan], overlay: &[TokenSpan], window: Range<usize>) -> Vec<TokenSpan> {
    let first = overlay.partition_point(|span| span.range.end <= window.start);
    let overlay = &overlay[first..];
    if overlay
        .first()
        .is_none_or(|span| span.range.start >= window.end)
    {
        return base.to_vec();
    }
    let mut out: Vec<TokenSpan> = Vec::with_capacity(base.len() + overlay.len());
    let mut push = |range: Range<usize>, class| {
        if range.is_empty() {
            return;
        }
        match out.last_mut() {
            Some(last) if last.range.end == range.start && last.class == class => {
                last.range.end = range.end;
            }
            _ => out.push(TokenSpan { range, class }),
        }
    };
    let mut next = 0;
    for span in base {
        let mut at = span.range.start;
        while at < span.range.end {
            while next < overlay.len() && overlay[next].range.end <= at {
                next += 1;
            }
            match overlay.get(next) {
                Some(over) if over.range.start < span.range.end => {
                    if over.range.start > at {
                        push(at..over.range.start, span.class);
                        at = over.range.start;
                    }
                    let end = over.range.end.min(span.range.end);
                    push(at..end, over.class);
                    at = end;
                }
                _ => {
                    push(at..span.range.end, span.class);
                    at = span.range.end;
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod delta_tests {
    use super::*;
    use serde_json::json;

    fn edit(start: usize, delete_count: usize, data: &[u32]) -> DeltaEdit {
        DeltaEdit {
            start,
            delete_count,
            data: data.to_vec(),
        }
    }

    const OLD: [u32; 10] = [0, 0, 3, 1, 0, 0, 4, 2, 2, 0];

    #[test]
    fn edits_replace_ranges_of_integers_of_the_old_array() {
        let new = apply_delta(&OLD, &[edit(5, 5, &[1, 1, 1, 3, 0, 0, 2, 2, 4, 0])]).unwrap();
        assert_eq!(new, [0, 0, 3, 1, 0, 1, 1, 1, 3, 0, 0, 2, 2, 4, 0]);
        // Several edits all describe the old array, whatever order they come in.
        let new = apply_delta(&OLD, &[edit(5, 0, &[9, 9, 9, 9, 9]), edit(0, 5, &[])]).unwrap();
        assert_eq!(new, [9, 9, 9, 9, 9, 0, 4, 2, 2, 0]);
        assert_eq!(apply_delta(&OLD, &[]).unwrap(), OLD);
        assert_eq!(
            apply_delta(&OLD, &[edit(0, 10, &[])]).unwrap(),
            Vec::<u32>::new()
        );
    }

    #[test]
    fn answers_that_break_the_rules_are_refused() {
        // Past the end.
        assert!(apply_delta(&OLD, &[edit(8, 5, &[])]).is_err());
        assert!(apply_delta(&OLD, &[edit(11, 0, &[])]).is_err());
        // Overlapping.
        assert!(apply_delta(&OLD, &[edit(0, 6, &[]), edit(5, 2, &[])]).is_err());
        // A partial token left over.
        assert!(apply_delta(&OLD, &[edit(0, 3, &[])]).is_err());
        assert!(apply_delta(&OLD, &[edit(0, 0, &[1, 2])]).is_err());
        // Overflowing arithmetic does not panic.
        assert!(apply_delta(&OLD, &[edit(usize::MAX, 2, &[])]).is_err());
    }

    #[test]
    fn the_integers_and_edits_of_responses_are_read_strictly() {
        assert_eq!(integers(&json!([1, 2, 3])), Some(vec![1, 2, 3]));
        assert_eq!(integers(&json!([1, -2])), None);
        assert_eq!(integers(&json!([1.5])), None);
        assert_eq!(integers(&json!([4294967296u64])), None);
        let edits = parse_delta(&json!({ "resultId": "x",
            "edits": [{ "start": 5, "deleteCount": 5, "data": [1, 2, 3, 4, 5] }, { "start": 0, "deleteCount": 0 }] }))
        .unwrap();
        assert_eq!(edits[0], edit(5, 5, &[1, 2, 3, 4, 5]));
        assert_eq!(edits[1], edit(0, 0, &[]));
        assert_eq!(
            parse_delta(&json!({ "data": [] })),
            None,
            "a full result is not a delta"
        );
        assert_eq!(parse_delta(&json!({ "edits": [{ "start": "a" }] })), None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::language_service::capabilities::PositionEncoding::{Utf8, Utf16};
    use serde_json::json;

    fn legend() -> SemanticLegend {
        SemanticLegend {
            token_types: ["keyword", "function", "variable", "unknownKind"]
                .map(String::from)
                .into(),
            token_modifiers: ["declaration", "readonly", "static", "defaultLibrary"]
                .map(String::from)
                .into(),
        }
    }

    fn spans(text: &str, data: Value, encoding: PositionEncoding) -> Vec<(String, &'static str)> {
        decode(&data, &legend(), text, encoding)
            .unwrap()
            .into_iter()
            .map(|s| (text[s.range].to_owned(), s.class.unwrap().name()))
            .collect()
    }

    #[test]
    fn relative_positions_become_absolute_byte_ranges() {
        let text = "fn main() {\n    let x = 1;\n}\n";
        // fn (0,0,2) keyword; main (0,3,4) function; let (1,4,3) keyword; x (0,4,1) variable
        let data = json!([0, 0, 2, 0, 0, 0, 3, 4, 1, 1, 1, 4, 3, 0, 0, 0, 4, 1, 2, 0]);
        assert_eq!(
            spans(text, data, Utf16),
            [
                ("fn".into(), "keyword"),
                ("main".into(), "function"),
                ("let".into(), "keyword"),
                ("x".into(), "variable")
            ]
        );
    }

    #[test]
    fn columns_are_counted_in_the_servers_encoding() {
        let text = "😀 fn";
        // In UTF-16 the emoji is two units, so "fn" starts at 3; in UTF-8 at 5.
        assert_eq!(
            spans(text, json!([0, 3, 2, 0, 0]), Utf16),
            [("fn".into(), "keyword")]
        );
        assert_eq!(
            spans(text, json!([0, 5, 2, 0, 0]), Utf8),
            [("fn".into(), "keyword")]
        );
    }

    #[test]
    fn modifiers_refine_the_class() {
        let text = "xs";
        // variable with readonly (bit 1) and static (bit 2) is a constant
        assert_eq!(
            spans(text, json!([0, 0, 2, 2, 0b110]), Utf16),
            [("xs".into(), "constant")]
        );
        assert_eq!(
            spans(text, json!([0, 0, 2, 1, 0b1000]), Utf16),
            [("xs".into(), "function.builtin")]
        );
    }

    #[test]
    fn unknown_kinds_overlaps_and_long_tokens_are_handled() {
        let text = "abc\nde";
        // unknownKind is skipped; the second token overlaps the first and is dropped;
        // the last one is longer than its line and is cut.
        let data = json!([0, 0, 1, 3, 0, 0, 0, 3, 0, 0, 0, 1, 1, 0, 0, 1, 0, 9, 1, 0]);
        assert_eq!(
            spans(text, data, Utf16),
            [("abc".into(), "keyword"), ("de".into(), "function")]
        );
    }

    #[test]
    fn broken_data_is_refused() {
        let text = "abc";
        let bad = |data: Value| decode(&data, &legend(), text, Utf16).is_err();
        assert!(bad(json!([0, 0, 1])), "not a multiple of five");
        assert!(bad(json!("nope")));
        assert!(bad(json!([0, 0, 1, 9, 0])), "type outside the legend");
        assert!(bad(json!([-1, 0, 1, 0, 0])), "negative");
        assert!(bad(json!([0, 0, 1.5, 0, 0])), "not an integer");
        assert!(
            bad(json!([
                4294967295u64,
                0,
                1,
                0,
                0,
                4294967295u64,
                0,
                1,
                0,
                0
            ])),
            "line overflow"
        );
        assert!(
            bad(json!([
                0,
                4294967295u64,
                1,
                0,
                0,
                0,
                4294967295u64,
                1,
                0,
                0
            ])),
            "column overflow"
        );
        assert_eq!(decode(&json!([]), &legend(), text, Utf16), Ok(vec![]));
        assert_eq!(
            decode(&json!([5, 0, 1, 0, 0]), &legend(), text, Utf16),
            Ok(vec![]),
            "line beyond the text"
        );
    }

    fn span(range: Range<usize>, class: Option<HighlightId>) -> TokenSpan {
        TokenSpan { range, class }
    }

    #[test]
    fn composing_draws_the_overlay_over_the_base_and_keeps_the_tiling() {
        let kw = Some(HighlightId::KEYWORD);
        let ty = Some(HighlightId::TYPE);
        let var = Some(HighlightId::VARIABLE);
        let base = [span(0..4, kw), span(4..8, None), span(8..12, ty)];
        let overlay = [span(2..6, var), span(10..11, var)];
        assert_eq!(
            compose(&base, &overlay, 0..12),
            [
                span(0..2, kw),
                span(2..6, var),
                span(6..8, None),
                span(8..10, ty),
                span(10..11, var),
                span(11..12, ty)
            ]
        );
    }

    #[test]
    fn composing_ignores_overlay_outside_the_window_and_joins_equal_neighbours() {
        let kw = Some(HighlightId::KEYWORD);
        let base = [span(10..20, None)];
        assert_eq!(
            compose(&base, &[span(0..5, kw), span(30..40, kw)], 10..20),
            base
        );
        assert_eq!(compose(&base, &[], 10..20), base);
        let joined = compose(&base, &[span(5..15, kw), span(15..30, kw)], 10..20);
        assert_eq!(joined, [span(10..20, kw)]);
    }
}
