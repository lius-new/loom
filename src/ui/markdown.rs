//! Markdown documentation laid out and drawn: wrapped text in several styles,
//! code blocks, lists, quotes and rules, sized to a given width.
//!
//! [`layout`] is pure: it places runs of text from a measuring function, so the
//! rules (wrapping, indents, spacing) are tested without a window. [`render`]
//! turns a layout into elements.
use crate::features::markdown::{Block, Inline};
use crate::ui::theme;
use lgui::core::{CursorIcon, EventPolicy, UiEventContext};
use lgui::prelude::{
    Element, SelectHandler, SelectableSegment, UiRect, VisualStyle, group, panel, selectable_text,
    text,
};

/// How a run of text looks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
    pub link: bool,
    /// 0 for body text, 1 to 6 for headings.
    pub heading: u8,
    /// Syntax class of a code block run, when highlighted.
    pub faint: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub text: String,
    pub style: Style,
    /// The link a run belongs to.
    pub url: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub runs: Vec<Run>,
    /// Horizontal rules: y, left, right.
    pub rules: Vec<(f32, f32, f32)>,
    /// Backgrounds of code blocks.
    pub code_boxes: Vec<UiRect>,
    /// Vertical bars beside block quotes.
    pub quote_bars: Vec<UiRect>,
    pub width: f32,
    pub height: f32,
}

pub struct Metrics<'a> {
    /// The width of `text` drawn in `style`.
    pub measure: &'a dyn Fn(&str, Style) -> f32,
    pub line_h: f32,
    /// Space between blocks.
    pub gap: f32,
    pub indent: f32,
}

struct Placer<'a> {
    m: &'a Metrics<'a>,
    runs: Vec<Run>,
    rules: Vec<(f32, f32, f32)>,
    code_boxes: Vec<UiRect>,
    quote_bars: Vec<UiRect>,
    max_width: f32,
}

#[derive(Clone)]
struct Word {
    text: String,
    style: Style,
    url: Option<String>,
    /// A space precedes this word.
    spaced: bool,
    /// Forced break before this word.
    newline: bool,
}

fn words(inlines: &[Inline], base: Style, url: Option<&str>, out: &mut Vec<Word>) {
    for inline in inlines {
        match inline {
            Inline::Text(text) => {
                let mut pending_space = text.starts_with(char::is_whitespace);
                for (n, word) in text.split_whitespace().enumerate() {
                    out.push(Word {
                        text: word.to_owned(),
                        style: base,
                        url: url.map(str::to_owned),
                        spaced: pending_space || n > 0,
                        newline: false,
                    });
                    pending_space = false;
                }
                if text.ends_with(char::is_whitespace) && !text.trim().is_empty() {
                    // A trailing space belongs to whatever follows.
                    out.push(Word {
                        text: String::new(),
                        style: base,
                        url: None,
                        spaced: true,
                        newline: false,
                    });
                }
            }
            Inline::Code(code) => out.push(Word {
                text: code.clone(),
                style: Style { code: true, ..base },
                url: url.map(str::to_owned),
                spaced: false,
                newline: false,
            }),
            Inline::Strong(inner) => words(inner, Style { bold: true, ..base }, url, out),
            Inline::Emphasis(inner) => words(
                inner,
                Style {
                    italic: true,
                    ..base
                },
                url,
                out,
            ),
            Inline::Link { label, url } => {
                words(label, Style { link: true, ..base }, Some(url), out)
            }
            Inline::Break => out.push(Word {
                text: String::new(),
                style: base,
                url: None,
                spaced: false,
                newline: true,
            }),
        }
    }
}

impl Placer<'_> {
    /// Places wrapped words from `(x, y)` within `width`; returns the height used.
    fn paragraph(&mut self, inlines: &[Inline], base: Style, x: f32, y: f32, width: f32) -> f32 {
        let mut list = Vec::new();
        words(inlines, base, None, &mut list);
        // A word glued to a trailing-space marker takes its space from it.
        let mut merged: Vec<Word> = Vec::new();
        let mut carry_space = false;
        for mut word in list {
            if word.text.is_empty() && !word.newline {
                carry_space = true;
                continue;
            }
            if carry_space {
                word.spaced = true;
                carry_space = false;
            }
            merged.push(word);
        }
        let line_h = self.m.line_h * if base.heading > 0 { 1.25 } else { 1.0 };
        let mut cursor = x;
        let mut top = y;
        let mut first_on_line = true;
        for word in merged {
            if word.newline {
                top += line_h;
                cursor = x;
                first_on_line = true;
                continue;
            }
            let space = if word.spaced && !first_on_line {
                (self.m.measure)(" ", word.style)
            } else {
                0.0
            };
            let w = (self.m.measure)(&word.text, word.style);
            if !first_on_line && cursor + space + w > x + width {
                top += line_h;
                cursor = x;
                first_on_line = true;
            }
            let at = if first_on_line {
                cursor
            } else {
                cursor + space
            };
            self.runs.push(Run {
                x: at,
                y: top,
                width: w,
                text: word.text,
                style: word.style,
                url: word.url,
            });
            cursor = at + w;
            first_on_line = false;
        }
        top + line_h - y
    }

    fn blocks(&mut self, blocks: &[Block], x: f32, mut y: f32, width: f32) -> f32 {
        let start = y;
        for (n, block) in blocks.iter().enumerate() {
            if n > 0 {
                y += self.m.gap;
            }
            y += self.block(block, x, y, width);
        }
        y - start
    }

    fn block(&mut self, block: &Block, x: f32, y: f32, width: f32) -> f32 {
        match block {
            Block::Paragraph(inlines) => self.paragraph(inlines, Style::default(), x, y, width),
            Block::Heading { level, content } => self.paragraph(
                content,
                Style {
                    bold: true,
                    heading: (*level).clamp(1, 6),
                    ..Style::default()
                },
                x,
                y,
                width,
            ),
            Block::Code { text, .. } => {
                let pad = 4.0;
                let mut top = y + pad;
                let style = Style {
                    code: true,
                    ..Style::default()
                };
                let mut right = x;
                for line in text.lines() {
                    let w = (self.m.measure)(line, style);
                    right = right.max(x + pad + w);
                    if !line.is_empty() {
                        self.runs.push(Run {
                            x: x + pad,
                            y: top,
                            width: w,
                            text: line.to_owned(),
                            style,
                            url: None,
                        });
                    }
                    top += self.m.line_h;
                }
                let box_right = (x + width).min(right + pad).max(x + 2.0 * pad);
                self.code_boxes.push(UiRect::new(
                    x,
                    y,
                    box_right.max(x + width.min(40.0)),
                    top + pad,
                ));
                top + pad - y
            }
            Block::Quote(inner) => {
                let bar = 3.0;
                let gap = 8.0;
                let used = self.blocks(inner, x + bar + gap, y, width - bar - gap);
                self.quote_bars.push(UiRect::new(x, y, x + bar, y + used));
                used
            }
            Block::List { start, items } => {
                let indent = self.m.indent;
                let mut used = 0.0;
                for (n, item) in items.iter().enumerate() {
                    let marker = match start {
                        Some(first) => format!("{}.", first + n as u64),
                        None => "\u{2022}".to_owned(),
                    };
                    let w = (self.m.measure)(&marker, Style::default());
                    self.runs.push(Run {
                        x: x + indent - w - 4.0,
                        y: y + used,
                        width: w,
                        text: marker,
                        style: Style {
                            faint: true,
                            ..Style::default()
                        },
                        url: None,
                    });
                    used += self.blocks(item, x + indent, y + used, width - indent);
                    used += 2.0;
                }
                used
            }
            Block::Rule => {
                let line_y = y + self.m.line_h / 2.0;
                self.rules.push((line_y, x, x + width));
                self.m.line_h
            }
        }
    }
}

/// Lays `blocks` out within `max_width`.
pub fn layout(blocks: &[Block], max_width: f32, metrics: &Metrics) -> Layout {
    let mut placer = Placer {
        m: metrics,
        runs: Vec::new(),
        rules: Vec::new(),
        code_boxes: Vec::new(),
        quote_bars: Vec::new(),
        max_width,
    };
    let height = placer.blocks(blocks, 0.0, 0.0, max_width);
    let right = placer
        .runs
        .iter()
        .map(|run| run.x + run.width)
        .chain(placer.code_boxes.iter().map(|b| b.right))
        .fold(0.0f32, f32::max);
    Layout {
        width: right.min(placer.max_width),
        height,
        runs: placer.runs,
        rules: placer.rules,
        code_boxes: placer.code_boxes,
        quote_bars: placer.quote_bars,
    }
}

/// The text style for a run.
pub fn text_style(style: Style, size: f32) -> lgui::prelude::TextStyle {
    let colour = if style.link {
        theme::c().accent
    } else if style.faint {
        theme::c().text_faint
    } else if style.heading > 0 {
        theme::c().text_bright
    } else {
        theme::c().text
    };
    let heading_size = match style.heading {
        0 => size,
        1 => size * 1.4,
        2 => size * 1.25,
        _ => size * 1.1,
    };
    if style.code {
        let colour = if style.link {
            colour
        } else {
            theme::c().text_bright
        };
        if style.bold {
            theme::mono_bold(colour, size)
        } else {
            theme::mono(colour, size)
        }
    } else if style.bold || style.heading > 0 {
        theme::sans_semibold(colour, heading_size)
    } else {
        theme::sans(colour, heading_size)
    }
}

/// Measures a run the way [`render`] draws it.
pub fn measure(text: &str, style: Style, size: f32) -> f32 {
    if style.code {
        text.chars().count() as f32 * crate::ui::fonts::editor().char_w * size
            / crate::ui::fonts::editor().size.max(1.0)
    } else {
        let weight = if style.bold || style.heading > 0 {
            600
        } else {
            400
        };
        let scale = match style.heading {
            0 => 1.0,
            1 => 1.4,
            2 => 1.25,
            _ => 1.1,
        };
        crate::ui::tabs::measure_sans(text, size * scale, weight)
    }
}

/// Called with a link's URL when it is clicked.
pub type LinkHandler = std::sync::Arc<dyn Fn(String) + Send + Sync>;

/// What the pointer can do with the drawn text. Without it the text is only
/// drawn.
#[derive(Clone, Default)]
pub struct Interaction {
    /// Links click through here; otherwise they are only coloured.
    pub on_link: Option<LinkHandler>,
    /// The text becomes selectable; told the selection, or `None` once cleared.
    pub on_select: Option<SelectHandler>,
}

/// Draws a layout with its top-left corner at `(left, top)`.
pub fn render(
    layout: &Layout,
    left: f32,
    top: f32,
    size: f32,
    line_h: f32,
    interaction: Option<&Interaction>,
) -> Element {
    let bounds = UiRect::new(
        left,
        top,
        left + layout.width.max(1.0),
        top + layout.height.max(1.0),
    );
    let mut out = group(bounds);
    for b in &layout.code_boxes {
        out = out.child(panel(
            UiRect::new(left + b.left, top + b.top, left + b.right, top + b.bottom),
            VisualStyle::filled(theme::c().active_line).radius(3.0),
        ));
    }
    for b in &layout.quote_bars {
        out = out.child(panel(
            UiRect::new(left + b.left, top + b.top, left + b.right, top + b.bottom),
            VisualStyle::filled(theme::c().border),
        ));
    }
    for (y, from, to) in &layout.rules {
        out = out.child(panel(
            UiRect::new(left + from, top + y, left + to, top + y + 1.0),
            VisualStyle::filled(theme::c().border),
        ));
    }
    let run_rect = |run: &Run| {
        UiRect::new(
            left + run.x,
            top + run.y,
            left + run.x + run.width + 6.0,
            top + run.y + line_h,
        )
    };
    let on_link = interaction.and_then(|i| i.on_link.as_ref());
    let selectable = interaction.and_then(|i| i.on_select.clone());
    // One surface draws every run and selects across them; links sit above it.
    if let Some(on_select) = &selectable {
        let segments = layout
            .runs
            .iter()
            .map(|run| {
                SelectableSegment::new(run_rect(run), run.text.clone(), text_style(run.style, size))
            })
            .collect();
        let on_select = on_select.clone();
        out = out.child(
            selectable_text(bounds, segments)
                .selection_color(theme::c().accent)
                .on_select(move |cx, text| on_select(cx, text)),
        );
    }
    for run in &layout.runs {
        let rect = run_rect(run);
        let label = text(rect, run.text.clone(), text_style(run.style, size));
        let (Some(url), Some(handler)) = (run.url.as_ref(), on_link) else {
            if selectable.is_none() {
                out = out.child(label);
            }
            continue;
        };
        let (url, handler) = (url.clone(), handler.clone());
        let hit = UiRect::new(rect.left, rect.top, rect.left + run.width, rect.bottom);
        let link = panel(hit, VisualStyle::default())
            .event_policy(EventPolicy {
                focus: false,
                ..EventPolicy::INTERACTIVE
            })
            .cursor(CursorIcon::Pointer)
            .on_pointer_down_with_button(|cx, _, _| cx.stop_propagation())
            .on_click(move |cx: &mut UiEventContext| {
                cx.stop_propagation();
                handler(url.clone());
            });
        // The surface already draws the text when selectable.
        out = out.child(if selectable.is_some() {
            link
        } else {
            link.child(label)
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::markdown::parse;

    /// One unit of width per character, code the same: easy to reason about.
    fn flat(text: &str, _: Style) -> f32 {
        text.chars().count() as f32
    }

    fn metrics() -> Metrics<'static> {
        Metrics {
            measure: &flat,
            line_h: 10.0,
            gap: 4.0,
            indent: 6.0,
        }
    }

    fn lay(source: &str, width: f32) -> Layout {
        layout(&parse(source), width, &metrics())
    }

    fn lines(layout: &Layout) -> Vec<String> {
        let mut rows: Vec<(f32, Vec<&Run>)> = Vec::new();
        for run in &layout.runs {
            match rows.iter_mut().find(|(y, _)| *y == run.y) {
                Some((_, row)) => row.push(run),
                None => rows.push((run.y, vec![run])),
            }
        }
        rows.sort_by(|a, b| a.0.total_cmp(&b.0));
        rows.into_iter()
            .map(|(_, row)| {
                row.iter()
                    .map(|r| r.text.as_str())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect()
    }

    #[test]
    fn words_wrap_at_the_width() {
        let l = lay("one two three four five", 9.0);
        assert_eq!(lines(&l), ["one two", "three", "four five"]);
        assert_eq!(l.height, 30.0);
    }

    #[test]
    fn a_word_wider_than_the_width_stands_alone() {
        let l = lay("a verylongword b", 5.0);
        assert_eq!(lines(&l), ["a", "verylongword", "b"]);
    }

    #[test]
    fn blocks_are_separated_by_the_gap() {
        let l = lay("one\n\ntwo", 20.0);
        assert_eq!(l.runs[0].y, 0.0);
        assert_eq!(l.runs[1].y, 14.0);
        assert_eq!(l.height, 24.0);
    }

    #[test]
    fn inline_styles_become_run_styles() {
        let l = lay("plain **bold** `code` [link](u)", 80.0);
        let styles: Vec<_> = l.runs.iter().map(|r| r.style).collect();
        assert!(styles[1].bold);
        assert!(styles[2].code);
        assert!(styles[3].link);
        assert_eq!(l.runs[3].url.as_deref(), Some("u"));
    }

    #[test]
    fn punctuation_after_a_style_stays_attached() {
        let l = lay("see `x`, then", 80.0);
        let xs: Vec<(String, f32)> = l.runs.iter().map(|r| (r.text.clone(), r.x)).collect();
        // `x` ends at 5 + 1 = 5.. then "," follows immediately.
        assert_eq!(xs[1].0, "x");
        assert_eq!(xs[2].0, ",");
        assert_eq!(xs[2].1, xs[1].1 + 1.0);
    }

    #[test]
    fn code_blocks_keep_their_lines_and_get_a_box() {
        let l = lay("```\nfn a() {\n    b();\n}\n```", 40.0);
        assert_eq!(l.code_boxes.len(), 1);
        assert_eq!(l.runs.len(), 3);
        assert_eq!(l.runs[1].text, "    b();");
        assert!(l.code_boxes[0].bottom >= 3.0 * 10.0);
    }

    #[test]
    fn lists_indent_their_items_and_carry_markers() {
        let l = lay("- one\n- two", 40.0);
        let texts: Vec<&str> = l.runs.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(texts, ["\u{2022}", "one", "\u{2022}", "two"]);
        assert_eq!(l.runs[1].x, 6.0);
        assert!(l.runs[0].x < l.runs[1].x);
        let ordered = lay("3. a", 40.0);
        assert_eq!(ordered.runs[0].text, "3.");
    }

    #[test]
    fn quotes_get_a_bar_and_an_indent() {
        let l = lay("> quoted", 40.0);
        assert_eq!(l.quote_bars.len(), 1);
        assert_eq!(l.runs[0].x, 11.0);
    }

    #[test]
    fn rules_span_the_width() {
        let l = lay("---", 30.0);
        assert_eq!(l.rules, [(5.0, 0.0, 30.0)]);
    }

    #[test]
    fn hard_breaks_start_a_new_line() {
        let l = lay("a  \nb", 40.0);
        assert_eq!(lines(&l), ["a", "b"]);
    }

    #[test]
    fn an_empty_document_has_no_height() {
        let l = lay("", 40.0);
        assert_eq!((l.height, l.runs.len()), (0.0, 0));
    }
}
