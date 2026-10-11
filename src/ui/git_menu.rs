//! Source Control menus: a changed file's context menu, and the commit
//! options menu beside the commit button.

use std::path::PathBuf;

use lgui::core::{CursorIcon, EventPolicy, UiEventContext, UiEventKind, UiFocusHandle};
use lgui::prelude::{Color, Element, ShadowStyle, State, UiRect, VisualStyle, panel, text};
use lgui::services::ServicesContextExt;

use crate::git::{DiffRequest, DiffTarget, GitStoreSnapshot};
use crate::git_actions;
use crate::state::{AppState, ExplorerContextTarget, ExplorerTargetKind};
use crate::theme;
use crate::ui::git_panel::{CONFLICTS, STAGED, UNTRACKED};

const MENU_W: f32 = 220.0;
const PAD: f32 = 4.0;
const ITEM_H: f32 = 22.0;
const SEP_H: f32 = 5.0;
const CHECK_W: f32 = 22.0;

#[derive(Clone, Debug, PartialEq)]
pub enum GitMenuTarget {
    /// A changed file, relative to `repository`, in drawer section `section`.
    File {
        repository: PathBuf,
        path: PathBuf,
        section: u8,
    },
    Commit,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GitMenu {
    /// Where the menu opens; it flips to stay inside the window.
    pub pos: (f32, f32),
    pub target: GitMenuTarget,
    pub hover: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MenuAction {
    OpenChanges,
    OpenFile,
    Stage,
    Unstage,
    Discard,
    CopyPath,
    CopyRelativePath,
    Reveal,
    ToggleAmend,
    ToggleSignoff,
    UndoCommit,
}

enum Entry {
    Item {
        label: &'static str,
        /// `Some` for a checkable item.
        checked: Option<bool>,
        action: MenuAction,
    },
    Separator,
}

fn item(label: &'static str, action: MenuAction) -> Entry {
    Entry::Item {
        label,
        checked: None,
        action,
    }
}

fn entries(target: &GitMenuTarget, app: &AppState) -> Vec<Entry> {
    match target {
        GitMenuTarget::File { section, .. } => {
            let mut entries = vec![
                item("Open Changes", MenuAction::OpenChanges),
                item("Open File", MenuAction::OpenFile),
                Entry::Separator,
            ];
            match *section {
                STAGED => entries.push(item("Unstage Changes", MenuAction::Unstage)),
                CONFLICTS => entries.push(item("Mark as Resolved", MenuAction::Stage)),
                _ => {
                    entries.push(item("Stage Changes", MenuAction::Stage));
                    entries.push(item(
                        if *section == UNTRACKED {
                            "Delete File"
                        } else {
                            "Discard Changes"
                        },
                        MenuAction::Discard,
                    ));
                }
            }
            entries.extend([
                Entry::Separator,
                item("Copy Path", MenuAction::CopyPath),
                item("Copy Relative Path", MenuAction::CopyRelativePath),
                item("Reveal in File Explorer", MenuAction::Reveal),
            ]);
            entries
        }
        GitMenuTarget::Commit => vec![
            Entry::Item {
                label: "Amend Last Commit",
                checked: Some(app.git_commit_amend),
                action: MenuAction::ToggleAmend,
            },
            Entry::Item {
                label: "Sign Off",
                checked: Some(app.git_commit_signoff),
                action: MenuAction::ToggleSignoff,
            },
            Entry::Separator,
            item("Undo Last Commit", MenuAction::UndoCommit),
        ],
    }
}

fn perform(
    action: MenuAction,
    target: &GitMenuTarget,
    cx: &mut UiEventContext,
    state: &State<AppState>,
    store: &State<GitStoreSnapshot>,
    editor_focus: &UiFocusHandle,
) {
    if let GitMenuTarget::File {
        repository,
        path,
        section,
    } = target
    {
        let absolute = repository.join(path);
        let paths = vec![path.clone()];
        match action {
            MenuAction::OpenChanges => {
                git_actions::open_diff(
                    state,
                    DiffRequest {
                        repository_root: repository.clone(),
                        path: path.clone(),
                        target: if *section == STAGED {
                            DiffTarget::HeadToIndex
                        } else {
                            DiffTarget::IndexToWorktree
                        },
                    },
                    *section == UNTRACKED,
                );
                editor_focus.focus();
            }
            MenuAction::OpenFile => {
                let mut focus = false;
                state.update(|app| {
                    focus = crate::workspace_actions::open_tree_file(app, absolute, 2);
                });
                if focus {
                    editor_focus.focus();
                }
            }
            MenuAction::Stage => {
                git_actions::stage_paths(state, store, paths);
            }
            MenuAction::Unstage => {
                git_actions::unstage_paths(state, store, paths);
            }
            MenuAction::Discard => {
                if *section == UNTRACKED {
                    git_actions::request_discard(state, store, Vec::new(), paths);
                } else {
                    git_actions::request_discard(state, store, paths, Vec::new());
                }
            }
            MenuAction::CopyPath | MenuAction::CopyRelativePath => {
                super::context_menu::copy_target_path(
                    state,
                    cx.application().clipboard().as_ref(),
                    &absolute,
                    action == MenuAction::CopyRelativePath,
                );
            }
            MenuAction::Reveal => super::context_menu::reveal_target(
                state,
                &ExplorerContextTarget {
                    path: absolute,
                    kind: ExplorerTargetKind::File,
                },
            ),
            MenuAction::ToggleAmend | MenuAction::ToggleSignoff | MenuAction::UndoCommit => {}
        }
        return;
    }
    match action {
        MenuAction::ToggleAmend => state.update(|app| app.git_commit_amend = !app.git_commit_amend),
        MenuAction::ToggleSignoff => {
            state.update(|app| app.git_commit_signoff = !app.git_commit_signoff)
        }
        MenuAction::UndoCommit => {
            git_actions::undo_last_commit(state, store);
        }
        _ => {}
    }
}

fn measure(s: &str) -> f32 {
    super::tabs::measure(s, theme::UI_SIZE, 400)
}

pub fn render(
    viewport: UiRect,
    menu: &GitMenu,
    state: State<AppState>,
    store: State<GitStoreSnapshot>,
    editor_focus: UiFocusHandle,
) -> Element {
    let app = state.get();
    let entries = entries(&menu.target, &app);
    let checkable = entries.iter().any(|entry| {
        matches!(
            entry,
            Entry::Item {
                checked: Some(_),
                ..
            }
        )
    });
    let label_left = 10.0 + if checkable { CHECK_W } else { 0.0 };
    let menu_w = entries
        .iter()
        .map(|entry| match entry {
            Entry::Item { label, .. } => label_left + measure(label) + 24.0,
            Entry::Separator => 0.0,
        })
        .fold(MENU_W, f32::max);
    let menu_h = PAD * 2.0
        + entries
            .iter()
            .map(|entry| match entry {
                Entry::Item { .. } => ITEM_H,
                Entry::Separator => SEP_H,
            })
            .sum::<f32>();
    let (x, y) = menu.pos;
    let x = if x + menu_w <= viewport.right {
        x
    } else {
        x - menu_w
    };
    let y = if y + menu_h <= viewport.bottom {
        y
    } else {
        y - menu_h
    };
    let x = x.clamp(viewport.left, (viewport.right - menu_w).max(viewport.left));
    let y = y.clamp(viewport.top, (viewport.bottom - menu_h).max(viewport.top));
    let card = UiRect::new(x, y, x + menu_w, y + menu_h);

    let close_state = state.clone();
    let overlay = panel(viewport, VisualStyle::default())
        .event_policy(EventPolicy::INTERACTIVE)
        .on_pointer_down_with_button(move |_cx, pointer, _button| {
            if !card.contains(pointer.point) {
                close_state.update(|app| app.git_menu = None);
            }
        });
    let clear_state = state.clone();
    let mut surface = theme::bordered(card, theme::c().surface, theme::c().border, 6.0, 1.0)
        .shadow(
            ShadowStyle::new(Color::BLACK)
                .alpha(theme::c().shadow_alpha)
                .offset(0.0, 2.0)
                .blur(3.0),
        )
        .event_policy(EventPolicy::INTERACTIVE)
        .on_event_capture(UiEventKind::PointerMove, move |_cx, _payload| {
            clear_state.try_update(|app| {
                let Some(menu) = app.git_menu.as_mut() else {
                    return false;
                };
                menu.hover.take().is_some()
            });
        });

    let mut y = card.top + PAD;
    let mut index = 0usize;
    for entry in entries {
        let Entry::Item {
            label,
            checked,
            action,
        } = entry
        else {
            let line_y = y + (SEP_H - 1.0) / 2.0;
            surface = surface.child(panel(
                UiRect::new(card.left + 10.0, line_y, card.right - 10.0, line_y + 1.0),
                VisualStyle::filled(theme::c().border),
            ));
            y += SEP_H;
            continue;
        };
        let rect = UiRect::new(card.left + 1.0, y, card.right - 1.0, y + ITEM_H);
        let hovered = menu.hover == Some(index);
        let idx = index;
        let hover_state = state.clone();
        let click_state = state.clone();
        let click_store = store.clone();
        let click_focus = editor_focus.clone();
        let target = menu.target.clone();
        let mut row = panel(
            rect,
            if hovered {
                VisualStyle::filled(theme::c().selection)
            } else {
                VisualStyle::default()
            },
        )
        .event_policy(EventPolicy::INTERACTIVE)
        .cursor(CursorIcon::Pointer)
        .on_event(UiEventKind::PointerMove, move |_cx, _payload| {
            hover_state.try_update(move |app| {
                let Some(menu) = app.git_menu.as_mut() else {
                    return false;
                };
                let changed = menu.hover != Some(idx);
                menu.hover = Some(idx);
                changed
            });
        })
        .on_click(move |cx: &mut UiEventContext| {
            click_state.update(|app| app.git_menu = None);
            perform(
                action,
                &target,
                cx,
                &click_state,
                &click_store,
                &click_focus,
            );
        });
        let color = if hovered {
            theme::c().text_bright
        } else {
            theme::c().text_soft
        };
        if checked == Some(true) {
            row = row.child(super::sidebar::drawer_icon(
                "git-menu.check",
                "check",
                UiRect::new(
                    rect.left + 10.0,
                    rect.top + 4.0,
                    rect.left + 24.0,
                    rect.top + 18.0,
                ),
                color,
            ));
        }
        row = row.child(text(
            UiRect::new(
                rect.left + label_left,
                rect.top,
                rect.right - 10.0,
                rect.bottom,
            ),
            label,
            theme::mono(color, theme::UI_SIZE),
        ));
        surface = surface.child(row);
        y += ITEM_H;
        index += 1;
    }
    overlay.child(surface)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::git_panel::UNSTAGED;

    fn labels(target: &GitMenuTarget, app: &AppState) -> Vec<&'static str> {
        entries(target, app)
            .into_iter()
            .filter_map(|entry| match entry {
                Entry::Item { label, .. } => Some(label),
                Entry::Separator => None,
            })
            .collect()
    }

    fn file(section: u8) -> GitMenuTarget {
        GitMenuTarget::File {
            repository: PathBuf::from("repo"),
            path: PathBuf::from("src/main.rs"),
            section,
        }
    }

    #[test]
    fn file_menus_offer_the_section_actions() {
        let app = AppState::new();
        assert!(labels(&file(STAGED), &app).contains(&"Unstage Changes"));
        assert!(!labels(&file(STAGED), &app).contains(&"Discard Changes"));
        let unstaged = labels(&file(UNSTAGED), &app);
        assert!(unstaged.contains(&"Stage Changes") && unstaged.contains(&"Discard Changes"));
        assert!(labels(&file(UNTRACKED), &app).contains(&"Delete File"));
        assert!(labels(&file(CONFLICTS), &app).contains(&"Mark as Resolved"));
    }

    #[test]
    fn commit_menu_reflects_the_options() {
        let mut app = AppState::new();
        app.git_commit_amend = true;
        let entries = entries(&GitMenuTarget::Commit, &app);
        let checked = entries
            .iter()
            .filter_map(|entry| match entry {
                Entry::Item {
                    label,
                    checked: Some(checked),
                    ..
                } => Some((*label, *checked)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(checked, [("Amend Last Commit", true), ("Sign Off", false)]);
    }
}
