//! Source Control drawer.

use std::path::PathBuf;

use lgui::core::{
    CursorIcon, EventPolicy, PointerButton, SemanticRole, Semantics, UiElement, UiEventContext,
    UiEventKind, UiEventPayload, UiFocusHandle, UiId, WheelUnit, clip,
};
use lgui::prelude::{Element, State, UiRect, VisualStyle, panel, text};

use crate::git::{
    ChangeKind, ConflictSide, DiffTarget, FileState, GitStoreSnapshot, RepositorySnapshot,
    RepositoryState, StashInfo,
};
use crate::state::AppState;
use crate::ui::components::input::{self, InputBinding, InputOptions, InputState, InputStyle};
use crate::ui::components::scrollbar;
use crate::ui::git_menu::{GitMenu, GitMenuTarget};
use crate::{git_actions, theme};

const TOPLINE_H: f32 = theme::TABS_H;
const PROMPT_H: f32 = 38.0;
const SECTION_H: f32 = 22.0;
const ROW_H: f32 = 20.0;

pub fn render(
    rect: UiRect,
    state: State<AppState>,
    store: State<GitStoreSnapshot>,
    editor_focus: UiFocusHandle,
    commit_focus: UiFocusHandle,
    commit_id: UiId,
) -> Element {
    let app = state.get();
    let resizing = app.resizing_git_sidebar;
    let git = store.get();
    let mut root = panel(rect, VisualStyle::filled(theme::c().sidebar));

    // Compact topline: the branch lives in the title bar badge, while this
    // drawer only exposes change and sync state.
    root = root.child(text(
        UiRect::new(
            rect.left + 12.0,
            rect.top,
            rect.right - 12.0,
            rect.top + TOPLINE_H,
        ),
        "CHANGES",
        theme::mono_bold(theme::c().text_soft, theme::SMALL).tracking(0.8),
    ));
    root = root.child(panel(
        UiRect::new(
            rect.left,
            rect.top + TOPLINE_H - 1.0,
            rect.right,
            rect.top + TOPLINE_H,
        ),
        VisualStyle::filled(theme::c().border),
    ));
    let Some(repository) = git.active() else {
        let message = if git.initializing {
            "Detecting Git and scanning repositories…".to_owned()
        } else {
            git.last_error.as_ref().map_or(
                "Open a Git repository to use Source Control.".to_owned(),
                |error| error.user_message(),
            )
        };
        root = root.child(text(
            UiRect::new(
                rect.left + 14.0,
                rect.top + TOPLINE_H + 18.0,
                rect.right - 14.0,
                rect.top + TOPLINE_H + 58.0,
            ),
            message,
            theme::sans(theme::c().text_dim, theme::UI_SIZE),
        ));
        return root.child(git_resize_handle(rect, state, resizing));
    };

    let prompt_top = rect.bottom - PROMPT_H;
    let prepared = git.presentation.repositories.get(&repository.id);
    let sections = prepared.map(|prepared| &prepared.sections);
    let mut banner_bottom = rect.top + TOPLINE_H;
    if let Some(label) = operation_label(repository) {
        let banner = UiRect::new(
            rect.left,
            banner_bottom,
            rect.right,
            banner_bottom + BANNER_H,
        );
        let conflicts = sections.is_some_and(|sections| !sections[CONFLICTS as usize].is_empty());
        root = root.child(operation_banner(
            banner,
            label,
            repository.repository_state,
            conflicts,
            git.operation.is_none(),
            state.clone(),
            store.clone(),
        ));
        banner_bottom = banner.bottom;
    }
    let content_rect = UiRect::new(rect.left, banner_bottom, rect.right - 8.0, prompt_top);
    let mut content = Vec::new();
    let section_height = |filter: u8, count: usize| {
        if count == 0 {
            0.0
        } else {
            SECTION_H
                + 10.0
                + if app.git_collapsed_sections.contains(&filter) {
                    0.0
                } else {
                    count as f32 * ROW_H
                }
        }
    };
    let content_height = 8.0
        + [CONFLICTS, STAGED, UNSTAGED, UNTRACKED]
            .into_iter()
            .map(|filter| {
                section_height(
                    filter,
                    sections.map_or(0, |sections| sections[filter as usize].len()),
                )
            })
            .sum::<f32>()
        + section_height(STASHES, repository.stashes.len());
    let max_scroll = (content_height - content_rect.height()).max(0.0);
    let scroll = app.git_scroll.clamp(0.0, max_scroll);
    let mut y = content_rect.top + 8.0;
    // Rows the pointer can hover, in content coordinates (before scrolling).
    let mut hover_targets = Vec::new();
    for (title, filter) in [
        ("CONFLICTS", CONFLICTS),
        ("STAGED", STAGED),
        ("UNSTAGED", UNSTAGED),
        ("UNTRACKED", UNTRACKED),
    ] {
        let files = sections.map_or(&[][..], |sections| sections[filter as usize].as_slice());
        if files.is_empty() {
            continue;
        }
        let collapsed = app.git_collapsed_sections.contains(&filter);
        let section_rect = UiRect::new(rect.left, y, content_rect.right, y + SECTION_H);
        hover_targets.push((section_rect, GitHover::Section(filter)));
        let buttons = if app.git_hover == Some(GitHover::Section(filter)) {
            section_actions(filter)
                .iter()
                .map(|&action| {
                    let action_state = state.clone();
                    let action_store = store.clone();
                    let paths = files.to_vec();
                    RowButton {
                        key: format!("git-section-{filter}-{action:?}"),
                        icon: action.icon(),
                        name: action.section_name(filter),
                        on_click: Box::new(move || {
                            action.run(&action_state, &action_store, filter, paths.clone());
                        }),
                    }
                })
                .collect()
        } else {
            Vec::new()
        };
        content.push(section_header(
            section_rect,
            filter,
            title,
            files.len(),
            collapsed,
            buttons,
            state.clone(),
        ));
        y += SECTION_H;
        if collapsed {
            y += 10.0;
            continue;
        }
        let rows_top = y;
        let visible = visible_rows(
            rows_top,
            content_rect.top + scroll,
            content_rect.bottom + scroll,
            files.len(),
        );
        y += files.len() as f32 * ROW_H;
        for index in visible {
            let path = &files[index];
            let file_state = &repository.files[path];
            let row_y = rows_top + index as f32 * ROW_H;
            let row_rect = UiRect::new(rect.left, row_y, content_rect.right, row_y + ROW_H);
            let hover = GitHover::File(filter, path.clone());
            let hovered = app.git_hover.as_ref() == Some(&hover);
            hover_targets.push((row_rect, hover));
            let target = if filter == STAGED {
                DiffTarget::HeadToIndex
            } else {
                DiffTarget::IndexToWorktree
            };
            let active = app
                .workspace
                .active_diff()
                .is_some_and(|diff| diff.matches(&repository.worktree_root, path, target));
            content.push(file_row(
                row_rect,
                path.clone(),
                file_state.clone(),
                filter,
                target,
                active,
                hovered,
                repository.worktree_root.clone(),
                state.clone(),
                store.clone(),
                editor_focus.clone(),
            ));
        }
        y += 10.0;
    }
    if !repository.stashes.is_empty() {
        let collapsed = app.git_collapsed_sections.contains(&STASHES);
        let section_rect = UiRect::new(rect.left, y, content_rect.right, y + SECTION_H);
        hover_targets.push((section_rect, GitHover::Section(STASHES)));
        content.push(section_header(
            section_rect,
            STASHES,
            "STASHES",
            repository.stashes.len(),
            collapsed,
            Vec::new(),
            state.clone(),
        ));
        y += SECTION_H;
        if !collapsed {
            let rows_top = y;
            let visible = visible_rows(
                rows_top,
                content_rect.top + scroll,
                content_rect.bottom + scroll,
                repository.stashes.len(),
            );
            y += repository.stashes.len() as f32 * ROW_H;
            for index in visible {
                let stash = &repository.stashes[index];
                let row_y = rows_top + index as f32 * ROW_H;
                let row_rect = UiRect::new(rect.left, row_y, content_rect.right, row_y + ROW_H);
                let hover = GitHover::Stash(stash.oid.clone());
                let hovered = app.git_hover.as_ref() == Some(&hover);
                hover_targets.push((row_rect, hover));
                content.push(stash_row(
                    row_rect,
                    stash,
                    hovered,
                    state.clone(),
                    store.clone(),
                ));
            }
        }
        y += 10.0;
    }
    let content_bottom = y;
    // One hit test per move finds the hovered row, so rows need no hover
    // state of their own and the pointer never leaves a stale highlight.
    let hover_state = state.clone();
    root = root.on_event_capture(UiEventKind::PointerMove, move |_cx, payload| {
        if let UiEventPayload::PointerMove { pointer } = payload {
            let point = pointer.point;
            let hover = content_rect
                .contains(point)
                .then(|| {
                    hover_targets.iter().find(|(rect, _)| {
                        point.y >= rect.top - scroll && point.y < rect.bottom - scroll
                    })
                })
                .flatten()
                .map(|(_, hover)| hover.clone());
            hover_state.try_update(move |app| {
                let changed = app.git_hover != hover;
                app.git_hover = hover;
                changed
            });
        }
    });
    let wheel_state = state.clone();
    root = root.on_event(UiEventKind::Wheel, move |_cx, payload| {
        if let UiEventPayload::Wheel { delta } = payload {
            let step = match delta.unit {
                WheelUnit::Lines => delta.y * ROW_H * 3.0,
                WheelUnit::Pixels => delta.y,
            };
            wheel_state.update(move |app| {
                app.git_scroll = (app.git_scroll - step).clamp(0.0, max_scroll);
            });
        }
    });
    let mut viewport = clip(content_rect, 0.0, -scroll);
    for element in content {
        viewport = viewport.child(element);
    }
    root = root.child(viewport);
    if max_scroll > 0.0 && (app.git_sidebar_hovered || app.git_scrollbar_dragging) {
        root = root.child(git_vertical_scrollbar(
            content_rect,
            content_bottom,
            scroll,
            state.clone(),
        ));
    }
    root = root.child(panel(
        UiRect::new(rect.left, prompt_top, rect.right, rect.bottom),
        VisualStyle::filled(theme::c().bg),
    ));
    root = root.child(panel(
        UiRect::new(rect.left, prompt_top, rect.right, prompt_top + 1.0),
        VisualStyle::filled(theme::c().border),
    ));
    let options_rect = UiRect::new(
        rect.right - 24.0,
        prompt_top + 5.0,
        rect.right - 6.0,
        rect.bottom - 5.0,
    );
    let commit_button_rect = UiRect::new(
        options_rect.left - 30.0,
        prompt_top + 5.0,
        options_rect.left - 2.0,
        rect.bottom - 5.0,
    );
    root = root.child(commit_input(
        UiRect::new(
            rect.left + 6.0,
            prompt_top,
            commit_button_rect.left - 4.0,
            rect.bottom,
        ),
        state.clone(),
        commit_focus,
        commit_id,
    ));
    root = root.child(commit_button(
        commit_button_rect,
        state.clone(),
        store.clone(),
    ));
    root = root.child(commit_options_button(options_rect, state.clone()));
    root = root.child(git_resize_handle(rect, state, resizing));
    root
}

fn visible_rows(
    rows_top: f32,
    viewport_top: f32,
    viewport_bottom: f32,
    count: usize,
) -> std::ops::Range<usize> {
    let start = ((viewport_top - rows_top) / ROW_H).floor().max(0.0) as usize;
    let end = ((viewport_bottom - rows_top) / ROW_H).ceil().max(0.0) as usize;
    start.saturating_sub(1).min(count)..end.saturating_add(1).min(count)
}

#[cfg(test)]
mod virtualization_tests {
    use super::*;

    #[test]
    fn scrollbar_is_separate_from_resize_handle_and_drags_outside_its_strip() {
        use lgui::application::{AppView, ApplicationContext};
        use lgui::core::{InputEvent, Point, PointerData, UiScale, dispatch_runtime_output};
        use lgui::session::UiSession;
        use std::sync::{Arc, Mutex};

        let exposed = Arc::new(Mutex::new(None::<State<AppState>>));
        let output = exposed.clone();
        let viewport = UiRect::new(0.0, 0.0, 300.0, 300.0);
        let view: AppView = Arc::new(move |cx| {
            let state = cx.state_with(AppState::new);
            *output.lock().unwrap() = Some(state.clone());
            let scroll = state.get().git_scroll;
            panel(viewport, VisualStyle::default())
                .child(git_vertical_scrollbar(
                    viewport,
                    1200.0,
                    scroll,
                    state.clone(),
                ))
                .child(git_resize_handle(viewport, state, false))
        });
        let mut session = UiSession::new();
        session.render_view(&view, viewport, UiScale::ONE);
        let state = exposed.lock().unwrap().clone().unwrap();
        let point = Point::new(viewport.right - 14.0, 12.0);
        assert_eq!(session.tree().cursor_at(point), None);
        assert_eq!(
            session
                .tree()
                .cursor_at(Point::new(viewport.right - 4.0, 12.0)),
            Some(CursorIcon::ResizeHorizontal)
        );
        let context = ApplicationContext::empty(Default::default());
        let mut send = |input| {
            let events = session.handle_input(input);
            dispatch_runtime_output(
                events,
                &context,
                &lgui::window::WindowId::new("git-drawer-scrollbar-test"),
                |action| session.handle_default_action(action),
                |_| {},
            );
            session.render_view(&view, viewport, UiScale::ONE);
        };
        send(InputEvent::PointerDown {
            pointer: PointerData::mouse(point),
            button: PointerButton::Left,
        });
        assert!(state.get().git_scrollbar_dragging);
        assert!(!state.get().resizing_git_sidebar);
        let outside = PointerData::mouse(Point::new(30.0, 260.0));
        send(InputEvent::PointerMove(outside));
        assert!(state.get().git_scroll > 0.0);
        send(InputEvent::PointerUp {
            pointer: outside,
            button: PointerButton::Left,
        });
        assert!(!state.get().git_scrollbar_dragging);
        let released = state.get().git_scroll;
        send(InputEvent::PointerMove(PointerData::mouse(point)));
        assert_eq!(state.get().git_scroll, released);
    }

    #[test]
    fn a_hundred_thousand_changes_only_build_the_viewport_rows() {
        let first = visible_rows(0.0, 0.0, 480.0, 100_000);
        let middle = visible_rows(0.0, 50_000.0 * ROW_H, 50_000.0 * ROW_H + 480.0, 100_000);
        let viewport_rows = (480.0 / ROW_H).ceil() as usize + 2;
        assert!(first.len() <= viewport_rows);
        assert!(middle.len() <= viewport_rows);
        assert!(middle.start > 100); // Entries after the old 100-file cap are reachable.
        let end = visible_rows(0.0, 100_000.0 * ROW_H, 100_000.0 * ROW_H + 480.0, 100_000);
        assert_eq!(end.end, 100_000);
    }
}

fn git_vertical_scrollbar(
    rect: UiRect,
    content_bottom: f32,
    scroll: f32,
    state: State<AppState>,
) -> Element {
    let content_top = rect.top + 8.0;
    let content_h = (content_bottom - content_top).max(1.0);
    let viewport_h = rect.height();
    let max_scroll = (content_bottom - rect.bottom).max(0.0);
    let track = UiRect::new(
        rect.right - 8.0 - scrollbar::SIZE,
        rect.top + 8.0,
        rect.right - 8.0,
        rect.bottom - 8.0,
    );
    let thumb_h = (track.height() * viewport_h / content_h)
        .max(scrollbar::MIN_THUMB_LENGTH)
        .min(track.height());
    let travel = track.height() - thumb_h;
    let thumb_top = if max_scroll == 0.0 {
        track.top
    } else {
        track.top + travel * (scroll / max_scroll)
    };
    let track_top = track.top;
    let drag_start = state.clone();
    let drag_move = state.clone();
    let drag_end = state;
    scrollbar::thumb(
        UiRect::new(
            track.left + scrollbar::INSET,
            thumb_top,
            track.right - scrollbar::INSET,
            thumb_top + thumb_h,
        ),
        0xb8,
    )
    .key("git-vertical-scrollbar-thumb")
    .event_policy(EventPolicy::INTERACTIVE)
    .on_pointer_down(move |_cx, pointer| {
        drag_start.update(move |app| {
            app.git_scrollbar_dragging = true;
            app.git_scrollbar_drag_offset = pointer.point.y - thumb_top;
        });
    })
    .on_pointer_drag(move |_cx, pointer| {
        drag_move.update(move |app| {
            if app.git_scrollbar_dragging && travel > 0.0 {
                let next = (pointer.point.y - track_top - app.git_scrollbar_drag_offset) / travel
                    * max_scroll;
                app.git_scroll = next.clamp(0.0, max_scroll);
            }
        });
    })
    .on_pointer_up(move |_cx, _pointer| {
        drag_end.update(|app| app.git_scrollbar_dragging = false);
    })
}

fn git_resize_handle(rect: UiRect, state: State<AppState>, resizing: bool) -> Element {
    if state.get().application_layout.is_some() {
        return lgui::prelude::group(rect);
    }
    let handle_rect = UiRect::new(rect.right - 8.0, rect.top, rect.right, rect.bottom);
    let drawer_left = rect.left;
    let drag_start = state.clone();
    let drag_move = state.clone();
    let drag_end = state;
    crate::ui::components::resize_handle::render(
        "git-resize-handle",
        handle_rect,
        UiRect::new(rect.right - 1.0, rect.top, rect.right, rect.bottom),
        CursorIcon::ResizeHorizontal,
        resizing,
    )
    .on_pointer_down_with_button(move |_cx, _pointer, button| {
        if button == PointerButton::Left {
            drag_start.update(|app| app.resizing_git_sidebar = true);
        }
    })
    .on_pointer_drag(move |_cx, pointer| {
        drag_move.update(move |app| {
            if app.resizing_git_sidebar {
                app.git_sidebar_w = (pointer.point.x - drawer_left)
                    .clamp(theme::SIDEBAR_MIN_W, theme::SIDEBAR_MAX_W);
            }
        });
    })
    .on_pointer_up(move |_cx, _pointer| {
        drag_end.update(|app| app.resizing_git_sidebar = false);
    })
}

fn commit_input(rect: UiRect, state: State<AppState>, focus: UiFocusHandle, id: UiId) -> Element {
    let placeholder = if state.get().git_commit_amend {
        "Amend message (empty keeps it)"
    } else {
        "Commit message (Enter)"
    };
    input::render(
        rect,
        id,
        focus,
        InputBinding::new(state, commit_input_state, commit_input_state_mut),
        InputOptions {
            label: "Commit message",
            placeholder,
            style: InputStyle {
                text: theme::mono(theme::c().text, theme::SMALL),
                placeholder: theme::mono(theme::c().text_faint, theme::SMALL),
                caret: theme::c().text_faint,
                selection: theme::c().accent,
            },
            on_submit: None,
            on_cancel: None,
            on_blur: None,
        },
    )
}

fn commit_input_state(app: &AppState) -> &InputState {
    &app.git_commit_input
}

fn commit_input_state_mut(app: &mut AppState) -> &mut InputState {
    &mut app.git_commit_input
}

/// Opens the commit options menu: amend, sign-off and undo.
fn commit_options_button(rect: UiRect, state: State<AppState>) -> Element {
    let app = state.get();
    let open = app
        .git_menu
        .as_ref()
        .is_some_and(|menu| menu.target == GitMenuTarget::Commit);
    // Amend and sign-off stay visible while the menu is closed.
    let color = if app.git_commit_amend || app.git_commit_signoff {
        theme::c().accent
    } else {
        theme::c().text_faint
    };
    let click_state = state.clone();
    panel(
        rect,
        if open {
            VisualStyle::filled(theme::c().active_line).radius(3.0)
        } else {
            VisualStyle::default().radius(3.0)
        },
    )
    .key("git-commit-options")
    .event_policy(EventPolicy::INTERACTIVE)
    .semantics(Semantics::new(SemanticRole::Button).name("Commit options"))
    .cursor(CursorIcon::Pointer)
    .on_click(move || {
        click_state.update(|app| {
            app.git_menu = Some(GitMenu {
                pos: (rect.left, rect.top),
                target: GitMenuTarget::Commit,
                hover: None,
            });
        });
    })
    .child(super::sidebar::drawer_icon(
        "git-commit-options-icon",
        "chevron-down",
        UiRect::new(
            rect.left + 2.0,
            rect.top + (rect.height() - 14.0) / 2.0,
            rect.left + 16.0,
            rect.top + (rect.height() + 14.0) / 2.0,
        ),
        color,
    ))
}

fn commit_button(rect: UiRect, state: State<AppState>, store: State<GitStoreSnapshot>) -> Element {
    let app = state.get();
    let has_message = app.git_commit_amend || !app.git_commit_input.text().trim().is_empty();
    let click_state = state.clone();
    let click_store = store;
    panel(
        rect,
        if has_message {
            VisualStyle::filled(theme::c().active_line).radius(3.0)
        } else {
            VisualStyle::default().radius(3.0)
        },
    )
    .key("git-commit-button")
    .event_policy(EventPolicy::INTERACTIVE)
    .semantics(
        Semantics::new(SemanticRole::Button).name(if app.git_commit_amend {
            "Amend last commit"
        } else {
            "Commit changes"
        }),
    )
    .cursor(CursorIcon::Pointer)
    .on_click(move || {
        git_actions::commit(&click_state, &click_store);
    })
    .child(super::sidebar::drawer_icon(
        "git-commit-button-icon",
        "check",
        theme::icon_rect(rect),
        if has_message {
            theme::c().text
        } else {
            theme::c().text_faint
        },
    ))
}

/// Drawer sections, in display order. Also the indices of
/// `RepositoryPresentation::sections` and `git_collapsed_sections` entries.
pub const CONFLICTS: u8 = 0;
pub const STAGED: u8 = 1;
pub const UNSTAGED: u8 = 2;
pub const UNTRACKED: u8 = 3;
/// Listed after the change sections; its rows come from the snapshot.
pub const STASHES: u8 = 4;

/// The drawer row under the pointer, which shows its action buttons.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitHover {
    Section(u8),
    File(u8, PathBuf),
    /// A stash, by commit id.
    Stash(String),
}

const BANNER_H: f32 = 30.0;

/// A collapsible section title with its count, and `buttons` on its right.
fn section_header(
    rect: UiRect,
    filter: u8,
    title: &'static str,
    count: usize,
    collapsed: bool,
    buttons: Vec<RowButton>,
    state: State<AppState>,
) -> Element {
    let chevron_id = match filter {
        CONFLICTS => "git-section-conflicts-chevron",
        STAGED => "git-section-staged-chevron",
        UNSTAGED => "git-section-unstaged-chevron",
        UNTRACKED => "git-section-untracked-chevron",
        _ => "git-section-stashes-chevron",
    };
    let section = panel(rect, VisualStyle::default())
        .key(format!("git-section-{filter}"))
        .event_policy(EventPolicy::INTERACTIVE)
        .cursor(CursorIcon::Pointer)
        .on_click(move || {
            state.update(move |app| {
                if !app.git_collapsed_sections.insert(filter) {
                    app.git_collapsed_sections.remove(&filter);
                }
            });
        })
        .child(super::sidebar::drawer_icon(
            chevron_id,
            if collapsed {
                "chevron-right"
            } else {
                "chevron-down"
            },
            theme::icon_rect(UiRect::new(
                rect.left + 9.0,
                rect.top + 5.0,
                rect.left + 21.0,
                rect.top + 17.0,
            )),
            theme::c().text_ghost,
        ))
        .child(text(
            UiRect::new(rect.left + 25.0, rect.top, rect.right - 38.0, rect.bottom),
            title,
            theme::mono(theme::c().text_ghost, theme::SMALL),
        ))
        .child(text(
            UiRect::new(rect.right - 38.0, rect.top, rect.right - 12.0, rect.bottom),
            count.to_string(),
            theme::mono_right(theme::c().text_ghost, theme::SMALL),
        ));
    if buttons.is_empty() {
        section
    } else {
        row_buttons(section, rect, rect.right - 40.0, buttons)
    }
}

/// A stash: its message, and apply, pop and drop while hovered.
fn stash_row(
    rect: UiRect,
    stash: &StashInfo,
    hovered: bool,
    state: State<AppState>,
    store: State<GitStoreSnapshot>,
) -> Element {
    let trailing_w = if hovered { BUTTON_W * 3.0 + 12.0 } else { 64.0 };
    let mut row = panel(
        rect,
        if hovered {
            VisualStyle::filled(theme::c().surface)
        } else {
            VisualStyle::default()
        },
    )
    .key(format!("git-stash-{}", stash.oid))
    .child(super::sidebar::drawer_icon(
        "git-stash-row-icon",
        "git-stash",
        UiRect::new(
            rect.left + 11.0,
            rect.top + (rect.height() - 12.0) / 2.0,
            rect.left + 23.0,
            rect.top + (rect.height() + 12.0) / 2.0,
        ),
        theme::c().text_ghost,
    ))
    .child(
        clip(
            UiRect::new(
                rect.left + 30.0,
                rect.top,
                rect.right - trailing_w,
                rect.bottom,
            ),
            0.0,
            0.0,
        )
        .child(text(
            UiRect::new(rect.left + 30.0, rect.top, rect.left + 2000.0, rect.bottom),
            stash.message.clone(),
            theme::mono(
                if hovered {
                    theme::c().text
                } else {
                    theme::c().text_dim
                },
                theme::SMALL,
            ),
        )),
    );
    if !hovered {
        return row.child(text(
            UiRect::new(rect.right - 62.0, rect.top, rect.right - 12.0, rect.bottom),
            format!("stash@{{{}}}", stash.index),
            theme::mono_right(theme::c().text_ghost, 9.0),
        ));
    }
    let buttons = [
        ("git-stash-apply", "Apply Stash", StashAction::Apply),
        ("git-stash-pop", "Pop Stash", StashAction::Pop),
        ("trash", "Drop Stash", StashAction::Drop),
    ]
    .into_iter()
    .map(|(icon, name, action)| {
        let action_state = state.clone();
        let action_store = store.clone();
        let oid = stash.oid.clone();
        RowButton {
            key: format!("git-stash-{}-{action:?}", stash.oid),
            icon,
            name,
            on_click: Box::new(move || match action {
                StashAction::Apply => {
                    git_actions::apply_stash(&action_state, &action_store, oid.clone(), false);
                }
                StashAction::Pop => {
                    git_actions::apply_stash(&action_state, &action_store, oid.clone(), true);
                }
                StashAction::Drop => {
                    git_actions::request_drop_stash(&action_state, &action_store, oid.clone());
                }
            }),
        }
    })
    .collect();
    row = row_buttons(row, rect, rect.right - 8.0, buttons);
    row
}

#[derive(Clone, Copy, Debug)]
enum StashAction {
    Apply,
    Pop,
    Drop,
}

/// The banner label for an operation in progress, with rebase progress.
fn operation_label(repository: &RepositorySnapshot) -> Option<String> {
    let label = match repository.repository_state {
        RepositoryState::MergeInProgress => "MERGING",
        RepositoryState::RebaseInProgress => "REBASING",
        RepositoryState::CherryPickInProgress => "CHERRY-PICKING",
        RepositoryState::RevertInProgress => "REVERTING",
        RepositoryState::Bisecting => "BISECTING",
        RepositoryState::Normal | RepositoryState::DetachedHead | RepositoryState::UnbornBranch => {
            return None;
        }
    };
    Some(match repository.operation_step {
        Some((step, total)) => format!("{label} {step}/{total}"),
        None => label.to_owned(),
    })
}

/// The operation in progress, with Continue (not while conflicts remain or
/// another Git command runs) and Abort.
fn operation_banner(
    rect: UiRect,
    label: String,
    operation: RepositoryState,
    conflicts: bool,
    idle: bool,
    state: State<AppState>,
    store: State<GitStoreSnapshot>,
) -> Element {
    let mut banner = panel(rect, VisualStyle::filled(theme::c().surface))
        .child(panel(
            UiRect::new(rect.left, rect.top, rect.left + 2.0, rect.bottom),
            VisualStyle::filled(theme::c().warning),
        ))
        .child(panel(
            UiRect::new(rect.left, rect.bottom - 1.0, rect.right, rect.bottom),
            VisualStyle::filled(theme::c().border),
        ));
    let mut right = rect.right - 8.0;
    let abort_label = if operation == RepositoryState::Bisecting {
        "End"
    } else {
        "Abort"
    };
    let abort_state = state.clone();
    let abort_store = store.clone();
    let (abort, abort_left) = text_button(right, rect, abort_label, false, idle, move || {
        git_actions::request_abort(&abort_state, &abort_store);
    });
    banner = banner.child(abort);
    right = abort_left - 6.0;
    // Bisect has no continue; its steps run from the terminal.
    if operation != RepositoryState::Bisecting {
        let (button, left) = text_button(
            right,
            rect,
            "Continue",
            true,
            idle && !conflicts,
            move || {
                git_actions::continue_operation(&state, &store);
            },
        );
        banner = banner.child(button);
        right = left - 6.0;
    }
    let label_w = super::tabs::measure(&label, theme::SMALL, 700) + 6.0;
    banner = banner.child(text(
        UiRect::new(
            rect.left + 12.0,
            rect.top,
            rect.left + 12.0 + label_w,
            rect.bottom,
        ),
        label,
        theme::mono_bold(theme::c().warning, theme::SMALL),
    ));
    if conflicts {
        banner = banner.child(text(
            UiRect::new(rect.left + 18.0 + label_w, rect.top, right, rect.bottom),
            "Resolve conflicts",
            theme::mono(theme::c().text_dim, theme::SMALL),
        ));
    }
    banner
}

/// A small text button ending at `right`, vertically centered in `bar`.
/// Returns it and its left edge.
fn text_button(
    right: f32,
    bar: UiRect,
    label: &'static str,
    primary: bool,
    enabled: bool,
    on_click: impl Fn() + Send + Sync + 'static,
) -> (Element, f32) {
    let width = super::tabs::measure(label, theme::SMALL, 600) + 16.0;
    let top = bar.top + (bar.height() - 20.0) / 2.0;
    let rect = UiRect::new(right - width, top, right, top + 20.0);
    let (fill, color) = match (primary, enabled) {
        (_, false) => (theme::c().sidebar, theme::c().text_ghost),
        (true, true) => (theme::c().accent, theme::c().on_accent),
        (false, true) => (theme::c().sidebar, theme::c().text_soft),
    };
    let mut style = theme::mono_bold(color, theme::SMALL);
    style.align = lgui::prelude::TextAlign::Center;
    let button = theme::bordered(rect, fill, theme::c().border, 3.0, 1.0)
        .key(format!("git-banner-{label}"))
        .event_policy(EventPolicy::INTERACTIVE)
        .semantics(Semantics::new(SemanticRole::Button).name(label))
        .cursor(if enabled {
            CursorIcon::Pointer
        } else {
            CursorIcon::NotAllowed
        })
        .on_click(move || {
            if enabled {
                on_click();
            }
        })
        .child(text(rect, label, style));
    (button, rect.left)
}

const BUTTON_W: f32 = 20.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RowAction {
    OpenFile,
    Stage,
    Unstage,
    Discard,
    /// Resolve a conflict with the checked-out side (`--ours`).
    KeepCurrent,
    /// Resolve a conflict with the side being merged in (`--theirs`).
    TakeIncoming,
}

/// Buttons on a section header, left to right.
fn section_actions(section: u8) -> &'static [RowAction] {
    match section {
        STAGED => &[RowAction::Unstage],
        UNSTAGED | UNTRACKED => &[RowAction::Discard, RowAction::Stage],
        // Marking every conflict resolved at once is too easy to regret.
        _ => &[],
    }
}

/// Buttons on a file row, left to right.
fn file_actions(section: u8) -> &'static [RowAction] {
    match section {
        STAGED => &[RowAction::OpenFile, RowAction::Unstage],
        CONFLICTS => &[
            RowAction::OpenFile,
            RowAction::KeepCurrent,
            RowAction::TakeIncoming,
            RowAction::Stage,
        ],
        _ => &[RowAction::OpenFile, RowAction::Discard, RowAction::Stage],
    }
}

impl RowAction {
    fn icon(self) -> &'static str {
        match self {
            Self::OpenFile => "go-to-file",
            Self::Stage => "plus",
            Self::Unstage => "minus",
            Self::Discard => "discard",
            Self::KeepCurrent => "arrow-left",
            Self::TakeIncoming => "arrow-right",
        }
    }

    fn section_name(self, section: u8) -> &'static str {
        match self {
            Self::OpenFile | Self::KeepCurrent | Self::TakeIncoming => self.file_name(section),
            Self::Stage => "Stage All Changes",
            Self::Unstage => "Unstage All Changes",
            Self::Discard if section == UNTRACKED => "Delete All Untracked Files",
            Self::Discard => "Discard All Changes",
        }
    }

    fn file_name(self, section: u8) -> &'static str {
        match self {
            Self::OpenFile => "Open File",
            Self::Stage if section == CONFLICTS => "Mark as Resolved",
            Self::Stage => "Stage Changes",
            Self::Unstage => "Unstage Changes",
            Self::Discard if section == UNTRACKED => "Delete File",
            Self::Discard => "Discard Changes",
            Self::KeepCurrent => "Keep Current (Ours)",
            Self::TakeIncoming => "Take Incoming (Theirs)",
        }
    }

    /// Run a Git action on `paths` from `section`. Opening a file needs the
    /// editor, so rows handle it themselves.
    fn run(
        self,
        state: &State<AppState>,
        store: &State<GitStoreSnapshot>,
        section: u8,
        paths: Vec<PathBuf>,
    ) {
        match self {
            Self::OpenFile => {}
            Self::Stage => {
                git_actions::stage_paths(state, store, paths);
            }
            Self::Unstage => {
                git_actions::unstage_paths(state, store, paths);
            }
            Self::Discard if section == UNTRACKED => {
                git_actions::request_discard(state, store, Vec::new(), paths);
            }
            Self::Discard => {
                git_actions::request_discard(state, store, paths, Vec::new());
            }
            Self::KeepCurrent | Self::TakeIncoming => {
                let side = if self == Self::KeepCurrent {
                    ConflictSide::Ours
                } else {
                    ConflictSide::Theirs
                };
                for path in paths {
                    git_actions::resolve_conflict(state, store, path, side);
                }
            }
        }
    }
}

struct RowButton {
    key: String,
    icon: &'static str,
    name: &'static str,
    on_click: Box<dyn Fn() + Send + Sync>,
}

/// Add `buttons` to `row`, right-aligned to end at `right`.
fn row_buttons(mut row: Element, rect: UiRect, right: f32, buttons: Vec<RowButton>) -> Element {
    let mut left = right - BUTTON_W * buttons.len() as f32;
    for button in buttons {
        let slot = UiRect::new(left, rect.top + 1.0, left + BUTTON_W, rect.bottom - 1.0);
        let on_click = button.on_click;
        row = row.child(
            Element::new(move |cx| {
                let style = if cx.context.interaction_flags(&cx.id).hovered {
                    VisualStyle::filled(theme::c().active_line).radius(3.0)
                } else {
                    VisualStyle::default()
                };
                UiElement::panel(cx.id, slot, style).children(cx.children)
            })
            .key(button.key)
            .event_policy(EventPolicy::INTERACTIVE)
            .semantics(Semantics::new(SemanticRole::Button).name(button.name))
            .cursor(CursorIcon::Pointer)
            .on_click(move |cx: &mut UiEventContext| {
                on_click();
                cx.stop_propagation();
            })
            .child(super::sidebar::drawer_icon(
                "git-row-button-icon",
                button.icon,
                UiRect::new(
                    slot.left + 3.0,
                    slot.top + (slot.height() - 14.0) / 2.0,
                    slot.left + 17.0,
                    slot.top + (slot.height() + 14.0) / 2.0,
                ),
                theme::c().text_muted,
            )),
        );
        left += BUTTON_W;
    }
    row
}

#[allow(clippy::too_many_arguments)]
fn file_row(
    rect: UiRect,
    path: PathBuf,
    file_state: FileState,
    section: u8,
    target: DiffTarget,
    active: bool,
    hovered: bool,
    repository_root: PathBuf,
    state: State<AppState>,
    store: State<GitStoreSnapshot>,
    editor_focus: UiFocusHandle,
) -> Element {
    let staged = section == STAGED;
    let open_state = state.clone();
    let open_repository = repository_root.clone();
    let open_relative_path = path.clone();
    let untracked = file_state.worktree == ChangeKind::Untracked;
    let focus = editor_focus.clone();
    let kind = file_state.display_kind();
    let (indicator, indicator_color) = if file_state.conflict.is_some() {
        ("!", theme::c().git.deleted)
    } else if staged {
        ("+", theme::c().git.staged)
    } else {
        match kind {
            ChangeKind::Added => ("+", theme::c().git.added),
            ChangeKind::Modified | ChangeKind::TypeChanged => ("~", theme::c().git.modified),
            ChangeKind::Deleted => ("-", theme::c().git.deleted),
            ChangeKind::Renamed => (">", theme::c().git.modified),
            ChangeKind::Copied => ("+", theme::c().git.added),
            ChangeKind::Untracked => ("?", theme::c().git.added),
            ChangeKind::Unmerged => ("!", theme::c().git.deleted),
            _ => (kind.indicator(), theme::c().text_dim),
        }
    };
    let stat = if staged {
        file_state.index_stat.as_ref()
    } else {
        file_state.worktree_stat.as_ref()
    };
    let diff_summary = stat.map_or_else(String::new, |stat| {
        if stat.binary {
            "binary".to_owned()
        } else {
            format!("+{} -{}", stat.additions, stat.deletions)
        }
    });
    let actions = file_actions(section);
    // Hovering swaps the diff summary for the row's action buttons.
    let trailing_w = if hovered {
        BUTTON_W * actions.len() as f32 + 12.0
    } else {
        52.0
    };
    let name = path.display().to_string();
    let name_left = rect.left + 30.0;
    let name_viewport = UiRect::new(name_left, rect.top, rect.right - trailing_w, rect.bottom);
    // Match the editor: lay the full single-line content out at its natural
    // width, then clip the viewport. Giving the text only the viewport width
    // makes the text engine choose path separators as word-break points and
    // can hide an entire trailing path segment at once.
    let name_width =
        name.chars().count() as f32 * theme::CHAR_W * (theme::SMALL / theme::CODE_SIZE) + 12.0;
    let name_content = UiRect::new(
        name_left,
        rect.top,
        name_left + name_width.max(name_viewport.width()),
        rect.bottom,
    );
    let row_key = format!("git-file-{target:?}-{}", path.display());
    let menu_state = state.clone();
    let menu_repository = repository_root.clone();
    let menu_path = path.clone();
    let row = Element::new(move |cx| {
        let style = if active {
            VisualStyle::filled(theme::c().active_line)
        } else if hovered {
            VisualStyle::filled(theme::c().surface)
        } else {
            VisualStyle::default()
        };
        UiElement::panel(cx.id, rect, style).children(cx.children)
    })
    .key(row_key.clone())
    .event_policy(EventPolicy::INTERACTIVE)
    .cursor(CursorIcon::Pointer)
    .on_click(move || {
        // A right-click that opened this row's menu is not a click.
        let mut menu_open = false;
        open_state.try_update(|app| {
            menu_open = app.git_menu.is_some();
            false
        });
        if menu_open {
            return;
        }
        git_actions::open_diff(
            &open_state,
            crate::git::DiffRequest {
                repository_root: open_repository.clone(),
                path: open_relative_path.clone(),
                target,
            },
            untracked,
        );
        focus.focus();
    })
    .on_pointer_down_with_button(move |_cx, pointer, button| {
        if button == PointerButton::Right {
            let target = GitMenuTarget::File {
                repository: menu_repository.clone(),
                path: menu_path.clone(),
                section,
            };
            menu_state.update(move |app| {
                app.git_menu = Some(GitMenu {
                    pos: (pointer.point.x, pointer.point.y),
                    target,
                    hover: None,
                });
            });
        }
    })
    .child(text(
        UiRect::new(rect.left + 12.0, rect.top, rect.left + 24.0, rect.bottom),
        indicator,
        theme::mono_bold(indicator_color, theme::SMALL),
    ))
    .child(clip(name_viewport, 0.0, 0.0).child(text(
        name_content,
        name,
        theme::mono(
            if active || hovered {
                theme::c().text
            } else {
                theme::c().text_dim
            },
            theme::SMALL,
        ),
    )));
    if !hovered {
        return row.child(text(
            UiRect::new(rect.right - 46.0, rect.top, rect.right - 12.0, rect.bottom),
            diff_summary,
            theme::mono_right(theme::c().text_ghost, 9.0),
        ));
    }
    let buttons = actions
        .iter()
        .map(|&action| {
            let action_state = state.clone();
            let action_store = store.clone();
            let action_path = path.clone();
            let open_path = repository_root.join(&path);
            let open_focus = editor_focus.clone();
            RowButton {
                key: format!("{row_key}-{action:?}"),
                icon: action.icon(),
                name: action.file_name(section),
                on_click: Box::new(move || {
                    if action == RowAction::OpenFile {
                        let mut focus = false;
                        let open_path = open_path.clone();
                        action_state.update(|app| {
                            focus = crate::workspace_actions::open_tree_file(app, open_path, 2);
                        });
                        if focus {
                            open_focus.focus();
                        }
                    } else {
                        action.run(
                            &action_state,
                            &action_store,
                            section,
                            vec![action_path.clone()],
                        );
                    }
                }),
            }
        })
        .collect();
    row_buttons(row, rect, rect.right - 8.0, buttons)
}

#[cfg(test)]
mod action_tests {
    use super::*;

    #[test]
    fn the_banner_names_the_operation_and_rebase_progress() {
        let mut repository = RepositorySnapshot {
            id: crate::git::RepositoryId(1),
            worktree_root: PathBuf::from("repo"),
            git_dir: PathBuf::from("repo/.git"),
            common_dir: PathBuf::from("repo/.git"),
            head: crate::git::HeadState::Branch("main".into()),
            upstream: None,
            ahead: 0,
            behind: 0,
            files: Default::default(),
            ignored: Default::default(),
            repository_state: RepositoryState::Normal,
            operation_step: None,
            head_oid: None,
            stashes: Default::default(),
            features: Default::default(),
            generation: 0,
        };
        assert_eq!(operation_label(&repository), None);
        repository.repository_state = RepositoryState::MergeInProgress;
        assert_eq!(operation_label(&repository).as_deref(), Some("MERGING"));
        repository.repository_state = RepositoryState::RebaseInProgress;
        repository.operation_step = Some((2, 5));
        assert_eq!(
            operation_label(&repository).as_deref(),
            Some("REBASING 2/5")
        );
    }

    #[test]
    fn each_section_offers_its_actions() {
        assert_eq!(section_actions(STAGED), [RowAction::Unstage]);
        assert_eq!(
            section_actions(UNSTAGED),
            [RowAction::Discard, RowAction::Stage]
        );
        assert!(section_actions(CONFLICTS).is_empty());
        assert_eq!(
            file_actions(CONFLICTS),
            [
                RowAction::OpenFile,
                RowAction::KeepCurrent,
                RowAction::TakeIncoming,
                RowAction::Stage
            ]
        );
        assert_eq!(RowAction::Stage.file_name(CONFLICTS), "Mark as Resolved");
        assert_eq!(RowAction::Discard.file_name(UNTRACKED), "Delete File");
        assert_eq!(
            RowAction::Discard.section_name(UNSTAGED),
            "Discard All Changes"
        );
    }
}
