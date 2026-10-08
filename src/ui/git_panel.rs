//! Source Control drawer.

use std::path::PathBuf;

use lgui::core::{
    CursorIcon, EventPolicy, PointerButton, SemanticRole, Semantics, UiElement, UiEventKind,
    UiEventPayload, UiFocusHandle, UiId, WheelUnit, clip,
};
use lgui::prelude::{Element, State, UiRect, VisualStyle, panel, text};

use crate::git::{ChangeKind, DiffTarget, FileState, GitStoreSnapshot};
use crate::state::AppState;
use crate::ui::components::input::{self, InputBinding, InputOptions, InputState, InputStyle};
use crate::ui::components::scrollbar;
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

    // Compact topline: the branch lives in the titlebar trigger, while this
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
    let content_rect = UiRect::new(
        rect.left,
        rect.top + TOPLINE_H,
        rect.right - 8.0,
        prompt_top,
    );
    let mut content = Vec::new();
    let prepared = git.presentation.repositories.get(&repository.id);
    let sections = prepared.map(|prepared| &prepared.sections);
    let content_height = 8.0
        + (0..4)
            .map(|filter| {
                let count = sections.map_or(0, |sections| sections[filter].len());
                if count == 0 {
                    0.0
                } else {
                    SECTION_H
                        + 10.0
                        + if app.git_collapsed_sections.contains(&(filter as u8)) {
                            0.0
                        } else {
                            count as f32 * ROW_H
                        }
                }
            })
            .sum::<f32>();
    let max_scroll = (content_height - content_rect.height()).max(0.0);
    let scroll = app.git_scroll.clamp(0.0, max_scroll);
    let mut y = content_rect.top + 8.0;
    for (title, filter) in [
        ("CONFLICTS", 0_u8),
        ("STAGED", 1),
        ("UNSTAGED", 2),
        ("UNTRACKED", 3),
    ] {
        let files = sections.map_or(&[][..], |sections| sections[filter as usize].as_slice());
        if files.is_empty() {
            continue;
        }
        let collapsed = app.git_collapsed_sections.contains(&filter);
        let section_icon_id = match filter {
            0 => "git-section-conflicts-chevron",
            1 => "git-section-staged-chevron",
            2 => "git-section-unstaged-chevron",
            _ => "git-section-untracked-chevron",
        };
        let section_icon = if collapsed {
            "chevron-right"
        } else {
            "chevron-down"
        };
        let section_state = state.clone();
        let section_rect = UiRect::new(rect.left, y, content_rect.right, y + SECTION_H);
        content.push(
            panel(section_rect, VisualStyle::default())
                .key(format!("git-section-{filter}"))
                .event_policy(EventPolicy::INTERACTIVE)
                .cursor(CursorIcon::Pointer)
                .on_click(move || {
                    section_state.update(move |app| {
                        if !app.git_collapsed_sections.insert(filter) {
                            app.git_collapsed_sections.remove(&filter);
                        }
                    });
                })
                .child(super::sidebar::drawer_icon(
                    section_icon_id,
                    section_icon,
                    UiRect::new(rect.left + 9.0, y + 5.0, rect.left + 21.0, y + 17.0),
                    theme::c().text_ghost,
                ))
                .child(text(
                    UiRect::new(
                        rect.left + 25.0,
                        y,
                        content_rect.right - 38.0,
                        y + SECTION_H,
                    ),
                    title,
                    theme::mono(theme::c().text_ghost, theme::SMALL),
                ))
                .child(text(
                    UiRect::new(
                        content_rect.right - 38.0,
                        y,
                        content_rect.right - 12.0,
                        y + SECTION_H,
                    ),
                    files.len().to_string(),
                    theme::mono_right(theme::c().text_ghost, theme::SMALL),
                )),
        );
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
            let target = if filter == 1 {
                DiffTarget::HeadToIndex
            } else {
                DiffTarget::IndexToWorktree
            };
            let active = app
                .workspace
                .active_diff()
                .is_some_and(|diff| diff.matches(&repository.worktree_root, path, target));
            content.push(file_row(
                UiRect::new(rect.left, row_y, content_rect.right, row_y + ROW_H),
                path.clone(),
                file_state.clone(),
                filter == 1,
                target,
                active,
                repository.worktree_root.clone(),
                state.clone(),
                store.clone(),
                editor_focus.clone(),
            ));
        }
        y += 10.0;
    }
    let content_bottom = y;
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
    let commit_button_rect = UiRect::new(
        rect.right - 34.0,
        prompt_top + 5.0,
        rect.right - 6.0,
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
    let handle_rect = UiRect::new(rect.right - 8.0, rect.top, rect.right, rect.bottom);
    let drawer_left = rect.left;
    let drag_start = state.clone();
    let drag_move = state.clone();
    let drag_end = state;
    panel(handle_rect, VisualStyle::default())
        .key("git-resize-handle")
        .event_policy(EventPolicy::INTERACTIVE)
        .cursor(CursorIcon::ResizeHorizontal)
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
        .child(panel(
            UiRect::new(rect.right - 1.0, rect.top, rect.right, rect.bottom),
            VisualStyle::filled(if resizing {
                theme::c().accent
            } else {
                theme::c().border
            }),
        ))
}

fn commit_input(rect: UiRect, state: State<AppState>, focus: UiFocusHandle, id: UiId) -> Element {
    input::render(
        rect,
        id,
        focus,
        InputBinding::new(state, commit_input_state, commit_input_state_mut),
        InputOptions {
            label: "Commit message",
            placeholder: "Commit message",
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

fn commit_button(rect: UiRect, state: State<AppState>, store: State<GitStoreSnapshot>) -> Element {
    let has_message = !state.get().git_commit_input.text().trim().is_empty();
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
    .semantics(Semantics::new(SemanticRole::Button).name("Commit changes"))
    .cursor(CursorIcon::Pointer)
    .on_click(move || {
        git_actions::commit(&click_state, &click_store);
    })
    .child(super::sidebar::drawer_icon(
        "git-commit-button-icon",
        "check",
        UiRect::new(
            rect.left + 7.0,
            rect.top + 7.0,
            rect.right - 7.0,
            rect.bottom - 7.0,
        ),
        if has_message {
            theme::c().text
        } else {
            theme::c().text_faint
        },
    ))
}

fn file_row(
    rect: UiRect,
    path: PathBuf,
    file_state: FileState,
    staged: bool,
    target: DiffTarget,
    active: bool,
    repository_root: PathBuf,
    state: State<AppState>,
    store: State<GitStoreSnapshot>,
    editor_focus: UiFocusHandle,
) -> Element {
    let open_state = state.clone();
    let open_repository = repository_root.clone();
    let open_relative_path = path.clone();
    let untracked = file_state.worktree == ChangeKind::Untracked;
    let focus = editor_focus;
    let action_state = state.clone();
    let action_store = store.clone();
    let action_path = path.clone();
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
    let name = path.display().to_string();
    let name_left = rect.left + 30.0;
    let name_viewport = UiRect::new(name_left, rect.top, rect.right - 52.0, rect.bottom);
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
    Element::new(move |cx| {
        let style = if active {
            VisualStyle::filled(theme::c().active_line)
        } else if cx.context.interaction_flags(&cx.id).hovered {
            VisualStyle::filled(theme::c().surface)
        } else {
            VisualStyle::default()
        };
        UiElement::panel(cx.id, rect, style).children(cx.children)
    })
    .key(row_key)
    .event_policy(EventPolicy::INTERACTIVE)
    .cursor(CursorIcon::Pointer)
    .on_click(move || {
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
    .child(text(
        UiRect::new(rect.left + 12.0, rect.top, rect.left + 24.0, rect.bottom),
        indicator,
        theme::mono_bold(indicator_color, theme::SMALL),
    ))
    .child(clip(name_viewport, 0.0, 0.0).child(text(
        name_content,
        name,
        theme::mono(
            if active {
                theme::c().text
            } else {
                theme::c().text_dim
            },
            theme::SMALL,
        ),
    )))
    .child(
        panel(
            UiRect::new(rect.right - 50.0, rect.top, rect.right, rect.bottom),
            VisualStyle::default(),
        )
        .event_policy(EventPolicy::INTERACTIVE)
        .on_click(move |cx: &mut lgui::core::UiEventContext| {
            if staged {
                git_actions::unstage_path(&action_state, &action_store, action_path.clone());
            } else {
                git_actions::stage_path(&action_state, &action_store, action_path.clone());
            }
            cx.stop_propagation();
        })
        .child(text(
            UiRect::new(rect.right - 46.0, rect.top, rect.right - 12.0, rect.bottom),
            diff_summary,
            theme::mono_right(theme::c().text_ghost, 9.0),
        )),
    )
}
