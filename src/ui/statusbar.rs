//! Status bar: cursor position, indent mode, encoding, and language badge,
//! grouped at the right edge. The terminal and Source Control toggles and the
//! branch badge, which opens the branch picker, sit on the left, followed by
//! the sync button.
//!
//! Items are laid out from measured text widths so every gap is uniform, and
//! each text rect carries a small right margin so glyphs are never clipped by
//! the rect bounds.
//!
//! With no active file there is no cursor position or language to report, so
//! only the universal editor settings (indent mode, encoding) remain.

use lgui::core::{
    CursorIcon, EventPolicy, IconStyle, SemanticRole, Semantics, UiElement, UiFocusHandle, UiId,
    precompiled,
};
use lgui::prelude::{Color, Element, State, TextStyle, UiRect, VisualStyle, panel, text};

use crate::git::{GitStoreSnapshot, RepositorySnapshot, RepositoryState};
use crate::git_actions::{self, SyncPlan};
use crate::input::action::Action;
use crate::state::AppState;
use crate::terminal_session::TerminalTabs;
use crate::theme;

/// Padding between the bar's right edge and the last item.
const EDGE_PAD: f32 = 12.0;
/// Uniform gap between adjacent items.
const GAP: f32 = 10.0;
/// Extra right margin inside each text rect so the last glyph is not clipped.
const TEXT_MARGIN: f32 = 6.0;

/// Natural width of `s` in the monospace status bar font at `weight`,
/// measured with the renderer's text system so it matches the rendered
/// pixels. Measuring with the default sans font underestimated monospace
/// labels, and a too-narrow rect wraps text such as `feat/keymap` at the `/`.
fn measure(s: &str, weight: i32) -> f32 {
    super::tabs::measure(s, theme::SMALL, weight)
}

pub fn render(
    rect: UiRect,
    state: State<AppState>,
    git_store: State<GitStoreSnapshot>,
    editor_focus: UiFocusHandle,
    terminal_focus: UiFocusHandle,
    terminal_tabs: State<TerminalTabs>,
) -> Element {
    let s = state.get();
    let git = git_store.get();

    let mut bar = panel(rect, VisualStyle::filled(theme::c().sidebar));

    let st = state.clone();
    let terminal_sessions = terminal_tabs;
    let editor_target = editor_focus;
    let terminal_target = terminal_focus;
    let top = rect.top + (rect.height() - theme::ICON_SIZE) / 2.0;
    let icon_r = UiRect::new(
        rect.left + 10.0,
        top,
        rect.left + 10.0 + theme::ICON_SIZE,
        top + theme::ICON_SIZE,
    );
    let color = if s.default_terminal_open() {
        theme::c().accent
    } else {
        theme::c().text_muted
    };
    bar = bar.child(
        panel(
            UiRect::new(rect.left, rect.top, rect.left + 34.0, rect.bottom),
            VisualStyle::default(),
        )
        .event_policy(EventPolicy {
            hover: true,
            press: true,
            focus: false,
        })
        .on_click(move || {
            super::terminal::toggle_panel(
                &st,
                &terminal_sessions,
                &editor_target,
                &terminal_target,
            );
        })
        .child(precompiled(
            UiElement::icon(UiId::new("statusbar.terminal"), icon_r, "terminal")
                .icon_style(IconStyle::new(color)),
        )),
    );
    // Right-aligned items. Without an active file, only the editor-wide
    // settings are shown (no Ln/Col position, no language badge).
    let mut items: Vec<(String, TextStyle)> = Vec::new();
    // A key sequence waiting for its next keystroke, e.g. `Ctrl+K …`.
    if !s.pending_keystrokes.is_empty() {
        items.push((
            format!(
                "{} …",
                crate::input::keystroke::Keystroke::sequence_label(&s.pending_keystrokes)
            ),
            theme::mono_bold(theme::c().accent, theme::SMALL),
        ));
    }
    if let Some(id) = s.workspace.active() {
        let m = s
            .workspace
            .meta(id)
            .expect("active document metadata exists");
        if s.workspace.is_missing_on_disk(id) {
            items.push((
                "Deleted on Disk — Save Recreates".to_string(),
                theme::mono_bold(theme::c().error, theme::SMALL),
            ));
        } else if s.workspace.has_disk_conflict(id) {
            items.push((
                "Changed on Disk".to_string(),
                theme::mono_bold(theme::c().warning, theme::SMALL),
            ));
        }
        if let Some(buffer) = s.workspace.active_editor() {
            let (line, col) = buffer.line_col();
            items.push((
                format!("Ln {}, Col {}", line + 1, col + 1),
                theme::mono(theme::c().text_muted, theme::SMALL),
            ));
            items.push((
                "Spaces: 2".to_string(),
                theme::mono(theme::c().text_dim, theme::SMALL),
            ));
            items.push((
                "UTF-8".to_string(),
                theme::mono(theme::c().text_dim, theme::SMALL),
            ));
        } else if s.workspace.is_diff(id) {
            items.push((
                "Text Diff".to_string(),
                theme::mono(theme::c().text_dim, theme::SMALL),
            ));
        }
        if s.workspace.is_file(id) || s.workspace.is_diff(id) {
            items.push((
                m.lang.badge().to_string(),
                theme::mono_bold(m.lang.badge_color(), theme::SMALL),
            ));
        }
    } else {
        items.push((
            "Spaces: 2".to_string(),
            theme::mono(theme::c().text_dim, theme::SMALL),
        ));
        items.push((
            "UTF-8".to_string(),
            theme::mono(theme::c().text_dim, theme::SMALL),
        ));
    }

    // Source Control panel toggle, beside the terminal toggle.
    let scm_slot = UiRect::new(rect.left + 34.0, rect.top, rect.left + 62.0, rect.bottom);
    let scm_state = state.clone();
    bar = bar.child(
        panel(scm_slot, VisualStyle::default())
            .event_policy(EventPolicy::INTERACTIVE)
            .semantics(Semantics::new(SemanticRole::Button).name("Toggle Source Control"))
            .on_click(move || {
                scm_state.update(|app| app.apply(Action::ToggleSourceControl));
            })
            .child(precompiled(
                UiElement::icon(
                    UiId::new("statusbar.source-control"),
                    UiRect::new(
                        scm_slot.left + 6.0,
                        top,
                        scm_slot.left + 6.0 + theme::ICON_SIZE,
                        top + theme::ICON_SIZE,
                    ),
                    "source-control",
                )
                .icon_style(IconStyle::new(if s.show_source_control {
                    theme::c().accent
                } else {
                    theme::c().text_muted
                })),
            )),
    );

    // Branch badge: opens the branch picker.
    let badge_left = scm_slot.right + 4.0;
    let (badge, mut git_right) = badge_element(
        rect,
        badge_left,
        git_badge_pieces(&git),
        s.branch_picker.is_some(),
    );
    let git_state = state.clone();
    let git_click_store = git_store.clone();
    bar = bar.child(
        badge
            .event_policy(EventPolicy::INTERACTIVE)
            .semantics(Semantics::new(SemanticRole::Button).name("Switch branch"))
            .cursor(CursorIcon::Pointer)
            .on_click(move || {
                super::branch_picker::open(&git_state, &git_click_store, badge_left);
            }),
    );

    // Sync button: publish, pull, push or fetch, whichever is due.
    if let Some(repository) = git.active()
        && let Some(plan) = git_actions::sync_plan(repository)
    {
        let enabled = git.operation.is_none();
        let color = if enabled {
            theme::c().text_muted
        } else {
            theme::c().text_ghost
        };
        let (sync, right) =
            badge_element(rect, git_right, sync_pieces(repository, plan, color), false);
        git_right = right;
        let sync_state = state.clone();
        let sync_store = git_store.clone();
        bar = bar.child(
            sync.event_policy(EventPolicy::INTERACTIVE)
                .semantics(Semantics::new(SemanticRole::Button).name(sync_name(plan)))
                .cursor(if enabled {
                    CursorIcon::Pointer
                } else {
                    CursorIcon::NotAllowed
                })
                .on_click(move || {
                    if enabled {
                        git_actions::sync(&sync_state, &sync_store);
                    }
                }),
        );
    }
    let mut left = git_right + GAP;
    if s.git_diff_loading.is_some() {
        bar = bar.child(text(
            UiRect::new(left, rect.top, left + 140.0, rect.bottom),
            "Loading diff…",
            theme::mono(theme::c().text_dim, theme::SMALL),
        ));
        left += 140.0 + GAP;
    }
    if let Some(operation) = &git.operation {
        bar = bar.child(text(
            UiRect::new(left, rect.top, left + 180.0, rect.bottom),
            operation.message.clone(),
            theme::mono(theme::c().text_dim, theme::SMALL),
        ));
        left += 180.0 + GAP;
    }
    if crate::editor::vim_input::is_active(&s) {
        for (label, style) in vim_items(&s.vim) {
            let width = measure(&label, style.weight) + TEXT_MARGIN;
            bar = bar.child(text(
                UiRect::new(left, rect.top, left + width, rect.bottom),
                label,
                style,
            ));
            left += width + GAP;
        }
    }

    let total = items
        .iter()
        .map(|(s, style)| measure(s, style.weight))
        .sum::<f32>()
        + GAP * (items.len().saturating_sub(1)) as f32;
    let mut x = rect.right - EDGE_PAD - total;
    for (label, style) in items {
        let w = measure(&label, style.weight);
        bar = bar.child(text(
            UiRect::new(x, rect.top, x + w + TEXT_MARGIN, rect.bottom),
            label,
            style,
        ));
        x += w + GAP;
    }

    // Hairline top border (matches the title bar's bottom border).
    bar = bar.child(panel(
        UiRect::new(rect.left, rect.top, rect.right, rect.top + 1.0),
        VisualStyle::filled(theme::c().border),
    ));

    bar
}

const BADGE_ICON: f32 = 12.0;
/// Gap between an icon and the text it labels.
const ICON_GAP: f32 = 4.0;
/// Gap between the badge's groups: branch, operation, sync state.
const GROUP_GAP: f32 = 8.0;

enum BadgePiece {
    Icon {
        id: &'static str,
        key: &'static str,
        color: Color,
    },
    Text {
        label: String,
        style: TextStyle,
    },
}

impl BadgePiece {
    fn width(&self) -> f32 {
        match self {
            Self::Icon { .. } => BADGE_ICON,
            Self::Text { label, style } => measure(label, style.weight),
        }
    }
}

/// The branch badge's contents, each with the gap before it: the branch
/// (`*` when dirty), then an operation in progress.
fn git_badge_pieces(git: &GitStoreSnapshot) -> Vec<(f32, BadgePiece)> {
    let Some(repository) = git.active() else {
        let color = theme::c().text_ghost;
        return vec![
            (
                0.0,
                BadgePiece::Icon {
                    id: "statusbar.git",
                    key: "git-branch",
                    color,
                },
            ),
            (
                ICON_GAP,
                BadgePiece::Text {
                    label: "No Git".into(),
                    style: theme::mono(color, theme::SMALL),
                },
            ),
        ];
    };
    let dirty = git
        .presentation
        .repositories
        .get(&repository.id)
        .is_some_and(|prepared| prepared.dirty_count > 0);
    let color = if dirty {
        theme::c().warning
    } else {
        theme::c().text_muted
    };
    let mut pieces = vec![
        (
            0.0,
            BadgePiece::Icon {
                id: "statusbar.git",
                key: "git-branch",
                color,
            },
        ),
        (
            ICON_GAP,
            BadgePiece::Text {
                label: format!(
                    "{}{}",
                    repository.branch_label(),
                    if dirty { "*" } else { "" }
                ),
                style: theme::mono(color, theme::SMALL),
            },
        ),
    ];
    if let Some(label) = operation_label(repository.repository_state) {
        pieces.push((
            GROUP_GAP,
            BadgePiece::Text {
                label: label.into(),
                style: theme::mono_bold(theme::c().warning, theme::SMALL),
            },
        ));
    }
    pieces
}

/// The sync button's contents: a publish icon for a branch with no
/// upstream, else the sync icon with the commits behind and ahead.
fn sync_pieces(
    repository: &RepositorySnapshot,
    plan: SyncPlan,
    color: Color,
) -> Vec<(f32, BadgePiece)> {
    if plan == SyncPlan::Publish {
        return vec![(
            0.0,
            BadgePiece::Icon {
                id: "statusbar.git.publish",
                key: "cloud-upload",
                color,
            },
        )];
    }
    let mut pieces = vec![(
        0.0,
        BadgePiece::Icon {
            id: "statusbar.git.sync",
            key: "sync",
            color,
        },
    )];
    for (count, id, key) in [
        (repository.behind, "statusbar.git.behind", "arrow-down"),
        (repository.ahead, "statusbar.git.ahead", "arrow-up"),
    ] {
        if count > 0 {
            pieces.push((ICON_GAP, BadgePiece::Icon { id, key, color }));
            pieces.push((
                1.0,
                BadgePiece::Text {
                    label: count.to_string(),
                    style: theme::mono(color, theme::SMALL),
                },
            ));
        }
    }
    pieces
}

/// What the sync button says it will do, for assistive technology.
fn sync_name(plan: SyncPlan) -> &'static str {
    match plan {
        SyncPlan::Publish => "Publish branch",
        SyncPlan::Fetch => "Fetch from remote",
        SyncPlan::Pull { then_push: true } => "Pull and push changes",
        SyncPlan::Pull { then_push: false } => "Pull changes",
        SyncPlan::Push => "Push changes",
    }
}

/// A status bar badge laid out from `pieces`, its left edge at `left`.
/// Returns it without a click handler, and its right edge.
fn badge_element(
    bar: UiRect,
    left: f32,
    pieces: Vec<(f32, BadgePiece)>,
    highlighted: bool,
) -> (Element, f32) {
    let pad = 7.0;
    let height = 16.0;
    let width = pad * 2.0
        + pieces
            .iter()
            .map(|(gap, piece)| gap + piece.width())
            .sum::<f32>()
        + TEXT_MARGIN;
    let slot = UiRect::new(left, bar.top, left + width, bar.bottom);
    let top = bar.top + (bar.height() - height) / 2.0;
    let visual = UiRect::new(slot.left, top, slot.right, top + height);
    let mut badge = panel(slot, VisualStyle::default()).child(panel(
        visual,
        if highlighted {
            VisualStyle::filled(theme::c().active_line).radius(3.0)
        } else {
            VisualStyle::default()
        },
    ));
    let mut x = visual.left + pad;
    for (gap, piece) in pieces {
        x += gap;
        let width = piece.width();
        badge = match piece {
            BadgePiece::Icon { id, key, color } => badge.child(precompiled(
                UiElement::icon(
                    UiId::new(id),
                    UiRect::new(
                        x,
                        top + (height - BADGE_ICON) / 2.0,
                        x + BADGE_ICON,
                        top + (height + BADGE_ICON) / 2.0,
                    ),
                    key,
                )
                .icon_style(IconStyle::new(color)),
            )),
            BadgePiece::Text { label, style } => badge.child(text(
                UiRect::new(x, visual.top, x + width + TEXT_MARGIN, visual.bottom),
                label,
                style,
            )),
        };
        x += width;
    }
    (badge, slot.right)
}

/// The badge label for an operation Git is in the middle of.
fn operation_label(state: RepositoryState) -> Option<&'static str> {
    match state {
        RepositoryState::MergeInProgress => Some("MERGING"),
        RepositoryState::RebaseInProgress => Some("REBASING"),
        RepositoryState::CherryPickInProgress => Some("CHERRY-PICKING"),
        RepositoryState::RevertInProgress => Some("REVERTING"),
        RepositoryState::Bisecting => Some("BISECTING"),
        RepositoryState::Normal | RepositoryState::DetachedHead | RepositoryState::UnbornBranch => {
            None
        }
    }
}

/// Vim's mode, then its command line, message or pending keys, and macro
/// recording, left to right.
fn vim_items(vim: &crate::vim::Vim) -> Vec<(String, TextStyle)> {
    use crate::vim::Mode;
    let status = vim.status();
    let mode_color = match vim.mode() {
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
