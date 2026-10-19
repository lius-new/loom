use super::presentation::project_window;
use super::{Source, TokenSpan, Window};
use crate::domain::{identity::FileId, language::builtin};
use crate::features::{highlighting::HighlightId, syntax::baseline::generate};
use loom_editor_core::text::{Edit, EditKind, EditorMut, Selection, TextBuffer};
use std::{ops::Range, sync::atomic::AtomicBool};
type H = HighlightId;

fn span(range: Range<usize>, class: Option<H>) -> TokenSpan {
    TokenSpan { range, class }
}

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
        let mut index = crate::features::syntax::test_support::index();
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
