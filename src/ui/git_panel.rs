//! Source Control drawer.

use std::path::PathBuf;

use lgui::core::{
    Color, CursorIcon, EventPolicy, PointerButton, SemanticRole, Semantics, UiEventKind,
    UiEventPayload, UiFocusHandle, UiId, WheelUnit, clip,
};
use lgui::prelude::{Element, State, UiRect, VisualStyle, panel, text};

use crate::git::{ChangeKind, FileState, GitStoreSnapshot};
use crate::state::AppState;
use crate::ui::components::input::{self, InputBinding, InputOptions, InputState, InputStyle};
use crate::{git_actions, theme};

const GIT_ADDED: Color = Color(0x38bdf8);
const GIT_MODIFIED: Color = Color(0xeab308);
const GIT_DELETED: Color = Color(0xf43f5e);
const GIT_STAGED: Color = Color(0x34d399);
const TOPLINE_H: f32 = theme::TABS_H;
const PROMPT_H: f32 = 38.0;
const SECTION_H: f32 = 22.0;
const ROW_H: f32 = 24.0;

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
    let mut root = panel(rect, VisualStyle::filled(theme::SIDEBAR));

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
        theme::mono(theme::ZINC_500, theme::SMALL),
    ));
    root = root.child(panel(
        UiRect::new(
            rect.left,
            rect.top + TOPLINE_H - 1.0,
            rect.right,
            rect.top + TOPLINE_H,
        ),
        VisualStyle::filled(theme::BORDER),
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
            theme::sans(theme::ZINC_500, theme::UI_SIZE),
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
    let mut y = content_rect.top + 8.0;
    for (title, filter) in [
        ("CONFLICTS", 0_u8),
        ("STAGED", 1),
        ("UNSTAGED", 2),
        ("UNTRACKED", 3),
    ] {
        let files = repository
            .files
            .iter()
            .filter(|(_, value)| match filter {
                0 => value.conflict.is_some(),
                1 => value.conflict.is_none() && value.index != ChangeKind::Unmodified,
                2 => {
                    value.conflict.is_none()
                        && value.worktree != ChangeKind::Unmodified
                        && value.worktree != ChangeKind::Untracked
                }
                _ => value.worktree == ChangeKind::Untracked,
            })
            .collect::<Vec<_>>();
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
                    theme::ZINC_700,
                ))
                .child(text(
                    UiRect::new(
                        rect.left + 25.0,
                        y,
                        content_rect.right - 38.0,
                        y + SECTION_H,
                    ),
                    title,
                    theme::mono(theme::ZINC_700, theme::SMALL),
                ))
                .child(text(
                    UiRect::new(
                        content_rect.right - 38.0,
                        y,
                        content_rect.right - 12.0,
                        y + SECTION_H,
                    ),
                    files.len().to_string(),
                    theme::mono_right(theme::ZINC_700, theme::SMALL),
                )),
        );
        y += SECTION_H;
        if collapsed {
            y += 10.0;
            continue;
        }
        for (path, file_state) in files.into_iter().take(100) {
            let absolute = repository.worktree_root.join(path);
            let active = app.workspace.active_path() == Some(absolute.as_path());
            content.push(file_row(
                UiRect::new(rect.left, y, content_rect.right, y + ROW_H),
                path.clone(),
                file_state.clone(),
                filter == 1,
                active,
                repository.worktree_root.clone(),
                state.clone(),
                store.clone(),
                editor_focus.clone(),
            ));
            y += ROW_H;
        }
        y += 10.0;
    }
    let content_bottom = y;
    let max_scroll = (content_bottom - content_rect.bottom).max(0.0);
    let scroll = app.git_scroll.clamp(0.0, max_scroll);
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
        VisualStyle::filled(theme::BG),
    ));
    root = root.child(panel(
        UiRect::new(rect.left, prompt_top, rect.right, prompt_top + 1.0),
        VisualStyle::filled(theme::BORDER),
    ));
    root = root.child(super::sidebar::drawer_icon(
        "git-commit-prompt-icon",
        "chevron-right",
        UiRect::new(
            rect.left + 10.0,
            prompt_top + 12.0,
            rect.left + 24.0,
            prompt_top + 26.0,
        ),
        theme::ZINC_600,
    ));
    let commit_button_rect = UiRect::new(
        rect.right - 34.0,
        prompt_top + 5.0,
        rect.right - 6.0,
        rect.bottom - 5.0,
    );
    root = root.child(commit_input(
        UiRect::new(
            rect.left + 30.0,
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
        rect.right - 7.0,
        rect.top + 8.0,
        rect.right - 3.0,
        rect.bottom - 8.0,
    );
    let thumb_h = (track.height() * viewport_h / content_h)
        .max(24.0)
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
    panel(
        UiRect::new(track.left, thumb_top, track.right, thumb_top + thumb_h),
        VisualStyle::filled(theme::ZINC_600).radius(2.0),
    )
    .key("git-vertical-scrollbar-thumb")
    .event_policy(EventPolicy::INTERACTIVE)
    .on_pointer_down(move |_cx, pointer| {
        drag_start.update(move |app| {
            app.git_scrollbar_dragging = true;
            app.git_scrollbar_drag_offset = pointer.point.y - thumb_top;
        });
    })
    .on_pointer_move(move |_cx, pointer| {
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
                theme::ACCENT
            } else {
                theme::BORDER
            }),
        ))
}

fn commit_input(
    rect: UiRect,
    state: State<AppState>,
    focus: UiFocusHandle,
    id: UiId,
) -> Element {
    input::render(
        rect,
        id,
        focus,
        InputBinding::new(state, commit_input_state, commit_input_state_mut),
        InputOptions {
            label: "Commit message",
            placeholder: "Commit message",
            style: InputStyle {
                text: theme::mono(theme::ZINC_200, theme::SMALL),
                placeholder: theme::mono(theme::ZINC_600, theme::SMALL),
                caret: theme::ZINC_600,
                selection: theme::ACCENT,
            },
        },
    )
}

fn commit_input_state(app: &AppState) -> &InputState {
    &app.git_commit_input
}

fn commit_input_state_mut(app: &mut AppState) -> &mut InputState {
    &mut app.git_commit_input
}

fn commit_button(
    rect: UiRect,
    state: State<AppState>,
    store: State<GitStoreSnapshot>,
) -> Element {
    let has_message = !state.get().git_commit_input.text().trim().is_empty();
    let click_state = state.clone();
    let click_store = store;
    panel(
        rect,
        if has_message {
            VisualStyle::filled(theme::ACTIVE_LINE).radius(3.0)
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
        "git-commit",
        UiRect::new(rect.left + 7.0, rect.top + 7.0, rect.right - 7.0, rect.bottom - 7.0),
        if has_message {
            theme::ZINC_200
        } else {
            theme::ZINC_600
        },
    ))
}

fn file_row(
    rect: UiRect,
    path: PathBuf,
    file_state: FileState,
    staged: bool,
    active: bool,
    repository_root: PathBuf,
    state: State<AppState>,
    store: State<GitStoreSnapshot>,
    editor_focus: UiFocusHandle,
) -> Element {
    let absolute = repository_root.join(&path);
    let open_state = state.clone();
    let open_path = absolute.clone();
    let focus = editor_focus;
    let action_state = state.clone();
    let action_store = store.clone();
    let action_path = path.clone();
    let kind = file_state.display_kind();
    let (indicator, indicator_color) = if file_state.conflict.is_some() {
        ("!", GIT_DELETED)
    } else if staged {
        ("+", GIT_STAGED)
    } else {
        match kind {
            ChangeKind::Added => ("+", GIT_ADDED),
            ChangeKind::Modified | ChangeKind::TypeChanged => ("~", GIT_MODIFIED),
            ChangeKind::Deleted => ("-", GIT_DELETED),
            ChangeKind::Renamed => (">", GIT_MODIFIED),
            ChangeKind::Copied => ("+", GIT_ADDED),
            ChangeKind::Untracked => ("?", GIT_ADDED),
            ChangeKind::Unmerged => ("!", GIT_DELETED),
            _ => (kind.indicator(), theme::ZINC_500),
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
    let name_width = name.chars().count() as f32
        * theme::CHAR_W
        * (theme::SMALL / theme::CODE_SIZE)
        + 12.0;
    let name_content = UiRect::new(
        name_left,
        rect.top,
        name_left + name_width.max(name_viewport.width()),
        rect.bottom,
    );
    panel(
        rect,
        if active {
            VisualStyle::filled(theme::ACTIVE_LINE)
        } else {
            VisualStyle::default()
        },
    )
        .event_policy(EventPolicy::INTERACTIVE)
        .cursor(CursorIcon::Pointer)
        .on_click(move || {
            if let Ok(contents) = std::fs::read_to_string(&open_path) {
                let path = open_path.clone();
                open_state.update(move |app| {
                    app.workspace.open_path(path, contents);
                });
                focus.focus();
            }
        })
        .child(text(
            UiRect::new(
                rect.left + 12.0,
                rect.top,
                rect.left + 24.0,
                rect.bottom,
            ),
            indicator,
            theme::mono_bold(indicator_color, theme::SMALL),
        ))
        .child(clip(name_viewport, 0.0, 0.0).child(text(
            name_content,
            name,
            theme::mono(
                if active {
                    theme::ZINC_200
                } else {
                    theme::ZINC_500
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
                UiRect::new(
                    rect.right - 46.0,
                    rect.top,
                    rect.right - 12.0,
                    rect.bottom,
                ),
                diff_summary,
                theme::mono_right(theme::ZINC_700, 9.0),
            )),
        )
}

