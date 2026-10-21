//! Status bar: cursor position, indent mode, encoding, and language badge,
//! grouped at the right edge. Panel toggles live in the title bar.
//!
//! Items are laid out from measured text widths so every gap is uniform, and
//! each text rect carries a small right margin so glyphs are never clipped by
//! the rect bounds.
//!
//! With no active file there is no cursor position or language to report, so
//! only the universal editor settings (indent mode, encoding) remain.

use lgui::core::{EventPolicy, SemanticRole, Semantics, UiId};
use lgui::prelude::{Element, TextStyle, UiRect, VisualStyle, panel, text};

use crate::ui::chrome::{ServerChip, StatusBinding, StatusCommand, StatusFrame};
use crate::ui::language_panel::Progress;
use crate::ui::theme;

/// Padding between the bar's right edge and the last item.
const EDGE_PAD: f32 = 12.0;
/// Uniform gap between adjacent items.
const GAP: f32 = 10.0;
/// Extra right margin inside each text rect so the last glyph is not clipped.
const TEXT_MARGIN: f32 = 6.0;
/// The white block holding the suggested action's icon, and the icon inside it.
const ACTION_W: f32 = 26.0;
const ACTION_ICON: f32 = 14.0;
/// Padding inside the server block.
const BLOCK_PAD: f32 = 10.0;
const RING: f32 = 12.0;
/// Between a block's progress ring and its text.
const INDICATOR_GAP: f32 = 6.0;

/// Natural width of `s` in the monospace status bar font at `weight`,
/// measured with the renderer's text system so it matches the rendered
/// pixels. Measuring with the default sans font underestimated monospace
/// labels, and a too-narrow rect wraps text such as `feat/keymap` at the `/`.
fn measure(s: &str, weight: i32) -> f32 {
    super::tabs::measure(s, theme::SMALL, weight)
}

/// The language server group: a block with the suggested action's icon, then the
/// server's state as a solid block, drawn in the flow of the status items.
struct Group {
    /// Where its placeholder sits among the items.
    index: usize,
    chip: ServerChip,
    chip_width: f32,
}

impl Group {
    fn action_width(&self) -> f32 {
        if self.chip.action.is_some() {
            ACTION_W
        } else {
            0.0
        }
    }

    fn width(&self) -> f32 {
        self.action_width() + self.chip_width
    }
}

/// The status items in order, with the language group's place held by an empty item.
struct Items {
    items: Vec<(String, TextStyle)>,
    /// The group and the diagnostics beside it: one click target for the panel.
    language: std::ops::Range<usize>,
    group: Option<Group>,
}

impl Items {
    fn width_of(&self, index: usize) -> f32 {
        match &self.group {
            Some(group) if group.index == index => group.width(),
            _ => {
                let (label, style) = &self.items[index];
                measure(label, style.weight)
            }
        }
    }

    fn total(&self) -> f32 {
        (0..self.items.len()).map(|i| self.width_of(i)).sum::<f32>()
            + GAP * self.items.len().saturating_sub(1) as f32
    }

    fn left(&self, rect: UiRect) -> f32 {
        rect.right - EDGE_PAD - self.total()
    }
}

/// The room a chip's progress ring takes before its text.
fn chip_indicator(chip: &ServerChip) -> f32 {
    match chip.progress {
        Progress::Percent(_) | Progress::Spinner => RING + INDICATOR_GAP,
        Progress::None => 0.0,
    }
}

/// Text that reads on `fill`: near-black on light fills, white on dark ones.
fn on_fill(fill: lgui::prelude::Color) -> lgui::prelude::Color {
    let lgui::prelude::Color(v) = fill;
    let (r, g, b) = ((v >> 16) & 255, (v >> 8) & 255, v & 255);
    if 0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32 > 150.0 {
        lgui::prelude::Color(0x1a1a1a)
    } else {
        lgui::prelude::Color(0xffffff)
    }
}

/// The measured layout anchors the language panel to the language items.
pub fn language_anchor(rect: UiRect, frame: &StatusFrame) -> Option<UiRect> {
    let items = status_items(frame);
    let mut x = items.left(rect);
    let mut span: Option<UiRect> = None;
    for index in 0..items.items.len() {
        let width = items.width_of(index);
        // The panel opens over the server group itself, not the diagnostics beside it.
        if items
            .group
            .as_ref()
            .is_some_and(|group| group.index == index)
        {
            return Some(UiRect::new(x, rect.top, x + width, rect.bottom));
        }
        if items.language.contains(&index) {
            let left = span.map_or(x, |span| span.left);
            span = Some(UiRect::new(left, rect.top, x + width, rect.bottom));
        }
        x += width + GAP;
    }
    span
}

pub fn render(rect: UiRect, state: StatusBinding) -> Element {
    let s = state.frame();

    let mut bar = panel(rect, VisualStyle::filled(theme::c().sidebar));

    let items = status_items(&s);

    let mut left = rect.left + EDGE_PAD;
    if s.diff_loading {
        bar = bar.child(text(
            UiRect::new(left, rect.top, left + 140.0, rect.bottom),
            "Loading diff…",
            theme::mono(theme::c().text_dim, theme::SMALL),
        ));
        left += 140.0 + GAP;
    }
    if let Some((mode, status)) = s.vim {
        for (label, style) in vim_status_items(mode, status) {
            let width = measure(&label, style.weight) + TEXT_MARGIN;
            bar = bar.child(text(
                UiRect::new(left, rect.top, left + width, rect.bottom),
                label,
                style,
            ));
            left += width + GAP;
        }
    }

    let mut x = items.left(rect);
    // The language group and its diagnostics are one click target that opens the panel.
    let mut language_span: Option<(f32, f32)> = None;
    let mut group_layer = None;
    for index in 0..items.items.len() {
        let w = items.width_of(index);
        let is_group = items.group.as_ref().is_some_and(|g| g.index == index);
        if items.language.contains(&index) {
            let (from, _) = language_span.unwrap_or((x, x));
            language_span = Some((from, x + w + TEXT_MARGIN));
        }
        if is_group {
            if let Some(group) = &items.group {
                group_layer = Some(render_group(rect, x, group, state.clone()));
            }
        } else {
            let (label, style) = items.items[index].clone();
            bar = bar.child(text(
                UiRect::new(x, rect.top, x + w + TEXT_MARGIN, rect.bottom),
                label,
                style,
            ));
        }
        x += w + GAP;
    }
    if let Some((from, to)) = language_span {
        let panel_state = state.clone();
        bar = bar.child(
            panel(
                UiRect::new(from - 4.0, rect.top, to + 4.0, rect.bottom),
                VisualStyle::default(),
            )
            .event_policy(EventPolicy::INTERACTIVE)
            .cursor(lgui::core::CursorIcon::Pointer)
            .semantics(Semantics::new(SemanticRole::Button).name("Language servers"))
            .on_click(move || {
                panel_state.run(StatusCommand::LanguagePanel);
            }),
        );
    }
    // Drawn over the panel's click target so the action block acts on its own.
    if let Some(layer) = group_layer {
        bar = bar.child(layer);
    }

    bar = bar.child(crate::ui::components::divider::top(rect));

    bar
}

fn status_items(s: &StatusFrame) -> Items {
    let mut items = Vec::new();
    if !s.pending.is_empty() {
        items.push((
            format!(
                "{} …",
                crate::features::keymap::keystroke::Keystroke::sequence_label(&s.pending)
            ),
            theme::mono_bold(theme::c().accent, theme::SMALL),
        ));
    }
    let language_start = items.len();
    for note in &s.language {
        let style = match note.level {
            crate::ui::chrome::NoteLevel::Quiet => theme::mono(theme::c().text_dim, theme::SMALL),
            crate::ui::chrome::NoteLevel::Warning => {
                theme::mono_bold(theme::c().warning, theme::SMALL)
            }
            crate::ui::chrome::NoteLevel::Error => theme::mono_bold(theme::c().error, theme::SMALL),
        };
        items.push((note.text.clone(), style));
    }
    // The diagnostics come first, so nothing follows the server group but the file's own items.
    let group = s.server.clone().map(|chip| {
        let chip_width = 2.0 * BLOCK_PAD + chip_indicator(&chip) + measure(&chip.text, 400);
        items.push((
            String::new(),
            theme::mono(theme::c().text_dim, theme::SMALL),
        ));
        Group {
            index: items.len() - 1,
            chip,
            chip_width,
        }
    });
    if let Some(file) = &s.file {
        if file.missing {
            items.push((
                "Deleted on Disk — Save Recreates".into(),
                theme::mono_bold(theme::c().error, theme::SMALL),
            ));
        } else if file.conflict {
            items.push((
                "Changed on Disk".into(),
                theme::mono_bold(theme::c().warning, theme::SMALL),
            ));
        }
        if let Some((line, col)) = file.cursor {
            items.push((
                format!("Ln {}, Col {}", line + 1, col + 1),
                theme::mono(theme::c().text_muted, theme::SMALL),
            ));
            items.push((
                "Spaces: 2".into(),
                theme::mono(theme::c().text_dim, theme::SMALL),
            ));
            items.push((
                "UTF-8".into(),
                theme::mono(theme::c().text_dim, theme::SMALL),
            ));
        } else if file.is_diff {
            items.push((
                "Text Diff".into(),
                theme::mono(theme::c().text_dim, theme::SMALL),
            ));
        }
        if file.is_file || file.is_diff {
            items.push((
                file.lang.badge().into(),
                theme::mono_bold(file.lang.badge_color(), theme::SMALL),
            ));
        }
    } else {
        items.push((
            "Spaces: 2".into(),
            theme::mono(theme::c().text_dim, theme::SMALL),
        ));
        items.push((
            "UTF-8".into(),
            theme::mono(theme::c().text_dim, theme::SMALL),
        ));
    }
    let language = language_start..items.len();
    Items {
        items,
        language,
        group,
    }
}

/// Vim's mode, then its command line, message or pending keys, and macro
/// recording, left to right.
fn vim_status_items(
    mode: loom_editor_core::vim::Mode,
    status: loom_editor_core::vim::Status,
) -> Vec<(String, TextStyle)> {
    use loom_editor_core::vim::Mode;
    let mode_color = match mode {
        Mode::Normal => theme::c().text_muted,
        Mode::Insert => theme::c().accent,
        Mode::Replace => theme::c().error,
        Mode::Visual(_) => theme::c().warning,
    };
    let mut items = vec![(
        format!("-- {} --", status.mode),
        theme::mono_bold(mode_color, theme::SMALL),
    )];
    if let Some(line) = status.command_line {
        items.push((line, theme::mono(theme::c().text_bright, theme::SMALL)));
    } else if let Some(message) = status.message {
        let color = if message.error {
            theme::c().error
        } else {
            theme::c().text_soft
        };
        items.push((message.text, theme::mono(color, theme::SMALL)));
    }
    if !status.pending.is_empty() {
        items.push((
            status.pending,
            theme::mono_bold(theme::c().accent, theme::SMALL),
        ));
    }
    if let Some(register) = status.recording {
        items.push((
            format!("recording @{register}"),
            theme::mono(theme::c().warning, theme::SMALL),
        ));
    }
    items
}

#[cfg(test)]
fn vim_items(vim: &loom_editor_core::vim::Vim) -> Vec<(String, TextStyle)> {
    vim_status_items(vim.mode(), vim.status())
}

/// The action block and the server's solid state block, full height, starting at `x`.
fn render_group(rect: UiRect, x: f32, group: &Group, state: StatusBinding) -> Element {
    let mut layer = lgui::prelude::group(rect);
    let chip = &group.chip;
    let fill = crate::ui::language_panel::tone_colour(chip.tone);
    let mut x = x;
    if let Some(action) = &chip.action {
        let block = UiRect::new(x, rect.top, x + ACTION_W, rect.bottom);
        let top = (rect.top + rect.bottom - ACTION_ICON) / 2.0;
        let left = block.left + (ACTION_W - ACTION_ICON) / 2.0;
        let mut element = panel(block, VisualStyle::filled(theme::c().surface));
        if action.interactive() {
            let command = action.clone();
            element = element
                .event_policy(EventPolicy::INTERACTIVE)
                .cursor(lgui::core::CursorIcon::Pointer)
                .semantics(Semantics::new(SemanticRole::Button).name(action.label()))
                .on_click(move || {
                    state.run(StatusCommand::ServerAction(command.clone()));
                });
        }
        layer = layer.child(element.child(crate::ui::tabs::icon(
            UiId::new("statusbar.language.action"),
            action.icon(),
            UiRect::new(left, top, left + ACTION_ICON, top + ACTION_ICON),
            fill,
        )));
        x += ACTION_W;
    }
    let block = UiRect::new(x, rect.top, x + group.chip_width, rect.bottom);
    let ink = on_fill(fill);
    let middle = (rect.top + rect.bottom) / 2.0;
    let mut left = block.left + BLOCK_PAD;
    let mut element = panel(block, VisualStyle::filled(fill)).event_policy(EventPolicy::NONE);
    match chip.progress {
        Progress::Percent(percent) => {
            element = element.child(crate::ui::components::progress_ring::render(
                UiId::new("statusbar.language.ring"),
                UiRect::new(left, middle - RING / 2.0, left + RING, middle + RING / 2.0),
                percent,
                ink,
            ));
            left += RING + INDICATOR_GAP;
        }
        Progress::Spinner => {
            element = element.child(crate::ui::components::loading_indicator::render(
                UiRect::new(left, middle - RING / 2.0, left + RING, middle + RING / 2.0),
                ink,
                "statusbar.language.spinner".to_owned(),
            ));
            left += RING + INDICATOR_GAP;
        }
        Progress::None => {}
    }
    layer.child(element.child(text(
        UiRect::new(left, block.top, block.right, block.bottom),
        chip.text.clone(),
        theme::mono(ink, theme::SMALL),
    )))
}

/// The action icon's block, which the details card opens above.
pub fn language_action_anchor(rect: UiRect, frame: &StatusFrame) -> Option<UiRect> {
    let items = status_items(frame);
    let group = items.group.as_ref()?;
    let mut x = items.left(rect);
    for index in 0..group.index {
        x += items.width_of(index) + GAP;
    }
    Some(UiRect::new(
        x,
        rect.top,
        x + if group.action_width() > 0.0 {
            group.action_width()
        } else {
            group.width()
        },
        rect.bottom,
    ))
}
