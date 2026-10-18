use super::presentation::{MAX_EDITS, MAX_REVISIONS, MAX_SPANS, Refused, project_window};
use super::treesitter::TreeIndex;
use super::{Source, TokenSpan, Window};
use crate::domain::identity::FileId;
use crate::domain::language::builtin;
use crate::features::highlighting::HighlightId;
use crate::features::syntax::baseline::generate;
use loom_editor_core::text::{Edit, EditKind, EditorMut, Selection, TextBuffer};
use std::ops::Range;
use std::sync::atomic::AtomicBool;

type H = HighlightId;

fn span(range: Range<usize>, class: Option<H>) -> TokenSpan {
    TokenSpan { range, class }
}

/// Edits one transaction and returns the journal entry that records it.
fn edit(buffer: &mut TextBuffer, edits: Vec<Edit>) -> loom_editor_core::text::TextRevision {
    let since = buffer.clone();
    EditorMut::new(buffer, &mut Selection::default(), Vec::new())
        .edit(EditKind::Other, edits, |_, _| Selection::default())
        .unwrap();
    buffer
        .revisions_since(since.version(), since.snapshot_identity())
        .unwrap()
        .remove(0)
}

fn classes(spans: &[TokenSpan]) -> Vec<(Range<usize>, Option<&'static str>)> {
    spans
        .iter()
        .map(|s| (s.range.clone(), s.class.map(H::name)))
        .collect()
}

/// `let ab = 12;` coloured keyword, plain, name, plain, number, plain.
fn sample() -> (TextBuffer, Vec<TokenSpan>) {
    let buffer = TextBuffer::new("let ab = 12;");
    let spans = vec![
        span(0..3, Some(H::KEYWORD)),
        span(3..4, None),
        span(4..6, Some(H::VARIABLE)),
        span(6..9, None),
        span(9..11, Some(H::NUMBER)),
        span(11..12, None),
    ];
    (buffer, spans)
}

#[test]
fn unchanged_text_keeps_its_class_and_moves_with_the_edits_before_it() {
    let (mut buffer, spans) = sample();
    let revision = edit(&mut buffer, vec![Edit::insert(0, "// ")]);
    let (window, projected) = project_window(0..12, &spans, &[revision], buffer.len()).unwrap();
    assert_eq!(window, 0..15);
    assert_eq!(buffer.text(), "// let ab = 12;");
    assert_eq!(
        classes(&projected),
        [
            // Nothing precedes the insertion, so it takes the class that follows
            // and joins the keyword.
            (0..6, Some("keyword")),
            (6..7, None),
            (7..9, Some("variable")),
            (9..12, None),
            (12..14, Some("number")),
            (14..15, None),
        ]
    );
}

#[test]
fn inserted_text_takes_the_class_before_it_and_neighbours_do_not_swallow_it() {
    let (mut buffer, spans) = sample();
    // Inside the name `ab`: the new characters join it.
    let revision = edit(&mut buffer, vec![Edit::insert(5, "XYZ")]);
    let (_, projected) = project_window(0..12, &spans, &[revision], buffer.len()).unwrap();
    assert_eq!(buffer.text(), "let aXYZb = 12;");
    assert_eq!(
        classes(&projected)[2],
        (4..9, Some("variable")),
        "one name, 2 + 3 characters"
    );

    // Between two spans, after the keyword: the left neighbour wins.
    let (mut buffer, spans) = sample();
    let revision = edit(&mut buffer, vec![Edit::insert(3, "_x")]);
    let (_, projected) = project_window(0..12, &spans, &[revision], buffer.len()).unwrap();
    assert_eq!(buffer.text(), "let_x ab = 12;");
    assert_eq!(classes(&projected)[0], (0..5, Some("keyword")));
    assert_eq!(
        classes(&projected)[1],
        (5..6, None),
        "the space is untouched"
    );

    // At the very end the last span's class continues.
    let (mut buffer, spans) = sample();
    let revision = edit(&mut buffer, vec![Edit::insert(12, " // x")]);
    let (_, projected) = project_window(0..12, &spans, &[revision], buffer.len()).unwrap();
    assert_eq!(projected.last().unwrap().range, 11..17);
    assert_eq!(projected.last().unwrap().class, None);
}

#[test]
fn deleted_and_replaced_text_leaves_no_trace() {
    let (mut buffer, spans) = sample();
    let revision = edit(&mut buffer, vec![Edit::replace(4..6, "")]);
    let (_, projected) = project_window(0..12, &spans, &[revision], buffer.len()).unwrap();
    assert_eq!(buffer.text(), "let  = 12;");
    assert_eq!(
        classes(&projected),
        [
            (0..3, Some("keyword")),
            (3..7, None),
            (7..9, Some("number")),
            (9..10, None)
        ],
        "the name is gone and its neighbours do not close over a zero-length gap"
    );

    let (mut buffer, spans) = sample();
    let revision = edit(&mut buffer, vec![Edit::replace(0..12, "x")]);
    let (_, projected) = project_window(0..12, &spans, &[revision], buffer.len()).unwrap();
    assert_eq!(
        classes(&projected),
        [(0..1, Some("keyword"))],
        "the whole text replaced"
    );

    let (mut buffer, spans) = sample();
    let revision = edit(&mut buffer, vec![Edit::replace(0..12, "")]);
    let (window, projected) = project_window(0..12, &spans, &[revision], buffer.len()).unwrap();
    assert!(window.is_empty() && projected.is_empty());
}

#[test]
fn several_edits_of_one_revision_and_several_revisions_compose() {
    let (mut buffer, spans) = sample();
    let first = edit(
        &mut buffer,
        vec![Edit::replace(0..3, "const"), Edit::replace(9..11, "1234")],
    );
    assert_eq!(buffer.text(), "const ab = 1234;");
    let second = edit(&mut buffer, vec![Edit::insert(0, "export ")]);
    let (window, projected) =
        project_window(0..12, &spans, &[first, second], buffer.len()).unwrap();
    assert_eq!(window, 0..23);
    assert_eq!(
        classes(&projected),
        [
            (0..12, Some("keyword")), // "export const": the insertion joined the first span
            (12..13, None),
            (13..15, Some("variable")),
            // The number was replaced as a whole; the new digits take the class
            // before them (plain) until the parser says otherwise.
            (15..23, None),
        ]
    );
}

#[test]
fn a_window_in_the_middle_follows_its_own_edges() {
    let (mut buffer, spans) = sample();
    let revision = edit(&mut buffer, vec![Edit::insert(0, "abcd")]);
    let inner = &spans[2..5];
    let (window, projected) = project_window(4..11, inner, &[revision], buffer.len()).unwrap();
    assert_eq!(window, 8..15);
    assert_eq!(projected.first().unwrap().range.start, 8);
    assert_eq!(projected.last().unwrap().range.end, 15);
    assert_eq!(
        classes(&projected),
        [
            (8..10, Some("variable")),
            (10..13, None),
            (13..15, Some("number"))
        ]
    );
}

#[test]
fn oversized_inputs_are_refused_instead_of_projected_slowly() {
    let many: Vec<TokenSpan> = (0..MAX_SPANS + 1)
        .map(|i| span(i..i + 1, if i % 2 == 0 { Some(H::KEYWORD) } else { None }))
        .collect();
    assert_eq!(
        project_window(0..many.len(), &many, &[], many.len()).unwrap_err(),
        Refused::Spans
    );
    let mut buffer = TextBuffer::new("a");
    let mut chain = Vec::new();
    for _ in 0..MAX_REVISIONS + 1 {
        chain.push(edit(&mut buffer, vec![Edit::insert(0, "b")]));
    }
    let one = [span(0..1, None)];
    assert_eq!(
        project_window(0..1, &one, &chain, buffer.len()).unwrap_err(),
        Refused::Revisions
    );
    let mut buffer = TextBuffer::new("x".repeat(200));
    let many: Vec<Edit> = (0..MAX_EDITS + 1)
        .map(|i| Edit::insert(i * 2, "y"))
        .collect();
    let revision = edit(&mut buffer, many);
    assert_eq!(
        project_window(0..200, &[span(0..200, None)], &[revision], buffer.len()).unwrap_err(),
        Refused::Edits
    );
}

struct Rng(u64);
impl Rng {
    fn next(&mut self, bound: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % bound as u64) as usize
    }
}

/// Applies a revision to a per-byte model of the classes, using only the numbers
/// in the journal, independently of the span arithmetic under test.
fn model_step(old: &[Option<H>], edits: &[loom_editor_core::text::RevisionEdit]) -> Vec<Option<H>> {
    let mut out = Vec::new();
    let mut at = 0;
    for edit in edits {
        out.extend_from_slice(&old[at..edit.old.start]);
        let class = match edit.old.start {
            0 => old.first().copied().flatten(),
            start => old[start - 1],
        };
        out.extend(std::iter::repeat_n(class, edit.new.len()));
        at = edit.old.end;
    }
    out.extend_from_slice(&old[at..]);
    out
}

fn per_byte(spans: &[TokenSpan], window: &Range<usize>) -> Vec<Option<H>> {
    let mut out = vec![None; window.len()];
    for span in spans {
        for at in span.range.clone() {
            out[at - window.start] = span.class;
        }
    }
    out
}

#[test]
fn projection_matches_a_byte_by_byte_model_for_random_edits() {
    let pieces = [
        "x", " ", "\n", "\"", "/*", "中", "😀", "\r\n", "fn ", "{", ";", "",
    ];
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let mut checked = 0;
    for round in 0..300 {
        let mut buffer = TextBuffer::new(generate("rust", 1536));
        let mut index = TreeIndex::default();
        let whole = Window {
            line: 0..buffer.len(),
            range: 0..buffer.len(),
        };
        let base = index
            .prepare(
                FileId::new(1),
                Source::new(&buffer),
                builtin("rust"),
                &[whole],
                &AtomicBool::new(false),
            )
            .unwrap()
            .remove(0)
            .spans
            .to_vec();
        // The window starts at the top, where the spans know everything before it;
        // a window further down only sees its own spans (tested separately).
        let window = {
            let end = (200 + rng.next(1200)).min(buffer.len());
            let end = (0..=end)
                .rev()
                .find(|&i| buffer.text().is_char_boundary(i))
                .unwrap();
            0..end
        };
        let inside: Vec<TokenSpan> = base
            .iter()
            .filter(|s| s.range.end > window.start && s.range.start < window.end)
            .map(|s| {
                span(
                    s.range.start.max(window.start)..s.range.end.min(window.end),
                    s.class,
                )
            })
            .collect();
        let mut model = per_byte(&base, &(0..buffer.len()));
        let mut chain = Vec::new();
        for _ in 0..1 + rng.next(4) {
            let len = buffer.len();
            let mut edits: Vec<Edit> = Vec::new();
            let mut cursor = 0;
            for _ in 0..1 + rng.next(4) {
                if cursor >= len {
                    break;
                }
                let mut start = cursor + rng.next(len - cursor);
                while !buffer.text().is_char_boundary(start) {
                    start += 1;
                }
                let mut end = (start + rng.next(8)).min(len);
                while !buffer.text().is_char_boundary(end) {
                    end += 1;
                }
                let text: String = (0..rng.next(3))
                    .map(|_| pieces[rng.next(pieces.len())])
                    .collect();
                if !(start == end && text.is_empty()) {
                    edits.push(Edit::replace(start..end, text));
                }
                cursor = end + 1;
            }
            let version = buffer.version();
            let since = buffer.clone();
            if EditorMut::new(&mut buffer, &mut Selection::default(), Vec::new())
                .edit(EditKind::Other, edits, |_, _| Selection::default())
                .is_err()
                || buffer.version() == version
            {
                continue;
            }
            let revision = buffer
                .revisions_since(since.version(), since.snapshot_identity())
                .unwrap()
                .remove(0);
            model = model_step(&model, &revision.edits);
            chain.push(revision);
        }
        if chain.is_empty() {
            continue;
        }
        let Ok((mapped, projected)) = project_window(window.clone(), &inside, &chain, buffer.len())
        else {
            continue;
        };
        // The result tiles the mapped window with no gaps, overlaps or empty spans.
        let mut cursor = mapped.start;
        for piece in &projected {
            assert_eq!(piece.range.start, cursor, "round {round}");
            assert!(!piece.range.is_empty(), "round {round}");
            cursor = piece.range.end;
        }
        assert_eq!(cursor, mapped.end, "round {round}");
        assert!(mapped.end <= buffer.len());
        // Wherever the window's text survived or was inserted, the class agrees
        // with the model's.
        let expected = &model[mapped.clone()];
        let actual = per_byte(&projected, &mapped);
        assert_eq!(actual, expected, "round {round}");
        checked += 1;
    }
    assert!(checked > 200, "{checked}");
}
