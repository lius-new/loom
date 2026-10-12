//! Source Control drawer.

use std::path::PathBuf;

use lgui::core::{
    CursorIcon, EventPolicy, PointerButton, SemanticRole, Semantics, UiElement, UiEventContext,
    UiEventKind, UiEventPayload, UiFocusHandle, UiId, WheelUnit, clip,
};
use lgui::prelude::{Element, State, UiRect, VisualStyle, panel, text};

use crate::git::{
    ChangeKind, CommitSummary, ConflictSide, DiffTarget, FileState, GitStoreSnapshot,
    RepositorySnapshot, RepositoryState, StashInfo, WorktreeInfo,
};
use crate::state::AppState;
use crate::ui::components::input::{self, InputBinding, InputOptions, InputState, InputStyle};
use crate::ui::components::scrollbar;
use crate::ui::git_menu::{GitMenu, GitMenuTarget};
use crate::{git_actions, theme};

const TOPLINE_H: f32 = theme::TABS_H;
const PROMPT_H: f32 = 32.0;
const SECTION_H: f32 = 22.0;
const ROW_H: f32 = 20.0;

/// The secondary sections stay anchored at the bottom. Expanded
/// details have a 45% height budget while there are changes to review, with
/// enough room for the section headers even in a short drawer.
fn drawer_regions(
    body: UiRect,
    changes_height: f32,
    details_height: f32,
    detail_headers: usize,
) -> (UiRect, UiRect, UiRect) {
    let available = body.height().max(0.0);
    let content_available = (available - PROMPT_H).max(0.0);
    let headers_height = detail_headers as f32 * SECTION_H;
    let limit = if changes_height > 8.0 {
        (content_available * 0.45).max(headers_height)
    } else {
        content_available
    };
    let height = details_height.min(limit).clamp(0.0, content_available);
    let top = body.bottom - height;
    // Short lists end naturally above the commit bar; long lists scroll in the
    // remaining space while the commit bar stays accessible below them.
    let changes_bottom = (body.top + changes_height).min((top - PROMPT_H).max(body.top));
    let prompt_bottom = (changes_bottom + PROMPT_H).min(top);
    (
        UiRect::new(body.left, body.top, body.right, changes_bottom),
        UiRect::new(body.left, changes_bottom, body.right, prompt_bottom),
        UiRect::new(body.left, top, body.right, body.bottom),
    )
}

#[derive(Clone, Copy)]
enum ScrollRegion {
    Changes,
    Details,
}

impl ScrollRegion {
    fn key(self) -> &'static str {
        match self {
            Self::Changes => "git-changes-viewport",
            Self::Details => "git-details-viewport",
        }
    }

    fn scroll(self, app: &AppState) -> f32 {
        match self {
            Self::Changes => app.git_scroll,
            Self::Details => app.git_details_scroll,
        }
    }

    fn set_scroll(self, app: &mut AppState, value: f32) {
        match self {
            Self::Changes => app.git_scroll = value,
            Self::Details => app.git_details_scroll = value,
        }
        app.git_commit_card = None;
    }

    fn drag(self, app: &AppState) -> (bool, f32) {
        match self {
            Self::Changes => (app.git_scrollbar_dragging, app.git_scrollbar_drag_offset),
            Self::Details => (
                app.git_details_scrollbar_dragging,
                app.git_details_scrollbar_drag_offset,
            ),
        }
    }

    fn set_drag(self, app: &mut AppState, dragging: bool, offset: f32) {
        match self {
            Self::Changes => {
                app.git_scrollbar_dragging = dragging;
                app.git_scrollbar_drag_offset = offset;
            }
            Self::Details => {
                app.git_details_scrollbar_dragging = dragging;
                app.git_details_scrollbar_drag_offset = offset;
            }
        }
    }
}

fn scroll_region(
    rect: UiRect,
    content: Vec<Element>,
    content_bottom: f32,
    scroll: f32,
    max_scroll: f32,
    state: State<AppState>,
    region: ScrollRegion,
) -> Element {
    if rect.height() <= 0.0 {
        return lgui::prelude::group(rect);
    }
    let mut clipped = clip(rect, 0.0, -scroll);
    for element in content {
        clipped = clipped.child(element);
    }
    let wheel_state = state.clone();
    let mut viewport = panel(rect, VisualStyle::default())
        .key(region.key())
        .child(clipped)
        .on_event(UiEventKind::Wheel, move |cx, payload| {
            if let UiEventPayload::Wheel { delta } = payload {
                let step = match delta.unit {
                    WheelUnit::Lines => delta.y * ROW_H * 3.0,
                    WheelUnit::Pixels => delta.y,
                };
                wheel_state.update(move |app| {
                    let next = (region.scroll(app) - step).clamp(0.0, max_scroll);
                    region.set_scroll(app, next);
                });
                cx.stop_propagation();
            }
        });
    let app = state.get();
    if max_scroll > 0.0 && (app.git_sidebar_hovered || region.drag(&app).0) {
        viewport = viewport.child(git_vertical_scrollbar(
            rect,
            content_bottom,
            scroll,
            state,
            region,
        ));
    }
    viewport
}

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
    let topline = UiRect::new(rect.left, rect.top, rect.right, rect.top + TOPLINE_H);
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
    let grouped = app.git_tree_view;
    let group_state = state.clone();
    root = root.child(row_buttons(
        lgui::prelude::group(topline),
        UiRect::new(
            topline.left,
            topline.top + 4.0,
            topline.right,
            topline.bottom - 4.0,
        ),
        topline.right - 8.0,
        vec![RowButton {
            key: "git-group-by-folder".into(),
            icon: if grouped { "list-flat" } else { "list-tree" },
            name: if grouped {
                "View as List"
            } else {
                "Group by Folder"
            },
            on_click: Box::new(move || {
                group_state.update(|app| app.git_tree_view = !app.git_tree_view);
            }),
        }],
    ));
    // With several repositories, name the active one and offer the others.
    if git.repositories.len() > 1
        && let Some(repository) = git.active()
    {
        let name = repository_name(&repository.worktree_root);
        let label_left =
            rect.left + 12.0 + super::tabs::measure("CHANGES", theme::SMALL, 700) + 14.0;
        let picker = UiRect::new(
            label_left - 4.0,
            topline.top + 4.0,
            (label_left + super::tabs::measure(&name, theme::SMALL, 400) + 26.0)
                .min(topline.right - 36.0),
            topline.bottom - 4.0,
        );
        let picker_state = state.clone();
        root = root.child(
            Element::new(move |cx| {
                let style = if cx.context.interaction_flags(&cx.id).hovered {
                    VisualStyle::filled(theme::c().active_line).radius(3.0)
                } else {
                    VisualStyle::default()
                };
                UiElement::panel(cx.id, picker, style).children(cx.children)
            })
            .key("git-repository-picker")
            .event_policy(EventPolicy::INTERACTIVE)
            .semantics(Semantics::new(SemanticRole::Button).name("Choose repository"))
            .cursor(CursorIcon::Pointer)
            .on_click(move || {
                picker_state.update(|app| {
                    app.git_menu = Some(GitMenu {
                        pos: (picker.left, picker.bottom + 2.0),
                        target: GitMenuTarget::Repositories,
                        hover: None,
                    });
                });
            })
            .child(
                clip(
                    UiRect::new(
                        picker.left + 4.0,
                        picker.top,
                        picker.right - 18.0,
                        picker.bottom,
                    ),
                    0.0,
                    0.0,
                )
                .child(text(
                    UiRect::new(
                        picker.left + 4.0,
                        picker.top,
                        picker.left + 1000.0,
                        picker.bottom,
                    ),
                    name,
                    theme::mono(theme::c().text_muted, theme::SMALL),
                )),
            )
            .child(super::sidebar::drawer_icon(
                "git-repository-picker-chevron",
                "chevron-down",
                UiRect::new(
                    picker.right - 16.0,
                    picker.top + (picker.height() - 12.0) / 2.0,
                    picker.right - 4.0,
                    picker.top + (picker.height() + 12.0) / 2.0,
                ),
                theme::c().text_dim,
            )),
        );
    }
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
    let body_rect = UiRect::new(
        rect.left,
        banner_bottom.min(rect.bottom),
        (rect.right - 8.0).max(rect.left),
        rect.bottom,
    );
    let mut content = Vec::new();
    // Every section's rows, laid out before drawing so the content height
    // and scroll range are known.
    let change_sections = [
        ("CONFLICTS", CONFLICTS),
        ("STAGED", STAGED),
        ("UNSTAGED", UNSTAGED),
        ("UNTRACKED", UNTRACKED),
    ]
    .map(|(title, filter)| {
        let files = sections.map_or(&[][..], |sections| sections[filter as usize].as_slice());
        (title, filter, files, change_rows(files, app.git_tree_view))
    });
    let history = app
        .git_history
        .as_ref()
        .filter(|history| history.repository == repository.worktree_root);
    let history_rows = history_rows(history, app.git_history_commit.as_ref());
    let section_height = |filter: u8, rows: usize| {
        SECTION_H
            + if app.git_collapsed_sections.contains(&filter) {
                0.0
            } else {
                rows as f32 * ROW_H
            }
    };
    let changes_height = 8.0
        + change_sections
            .iter()
            .filter(|(_, _, files, _)| !files.is_empty())
            .map(|(_, filter, _, rows)| section_height(*filter, rows.len()) + 10.0)
            .sum::<f32>();
    let details_height = if repository.stashes.is_empty() {
        0.0
    } else {
        section_height(STASHES, repository.stashes.len())
    } + if repository.worktrees.len() > 1 {
        section_height(WORKTREES, repository.worktrees.len() * 2)
    } else {
        0.0
    } + section_height(HISTORY, history_rows.len());
    let detail_headers = 1
        + usize::from(!repository.stashes.is_empty())
        + usize::from(repository.worktrees.len() > 1);
    let (content_rect, prompt_rect, details_rect) =
        drawer_regions(body_rect, changes_height, details_height, detail_headers);
    let prompt_top = prompt_rect.top;
    let prompt_bottom = prompt_rect.bottom;
    let max_scroll = (changes_height - content_rect.height()).max(0.0);
    let details_max_scroll = (details_height - details_rect.height()).max(0.0);
    let scroll = app.git_scroll.clamp(0.0, max_scroll);
    let mut y = content_rect.top + 8.0;
    // Rows the pointer can hover, in content coordinates (before scrolling).
    let mut hover_targets = Vec::new();
    for (title, filter, files, rows) in &change_sections {
        let (title, filter) = (*title, *filter);
        if files.is_empty() {
            continue;
        }
        let collapsed = app.git_collapsed_sections.contains(&filter);
        let section_rect = UiRect::new(rect.left, y, content_rect.right, y + SECTION_H);
        hover_targets.push((section_rect, GitHover::Section(filter)));
        let buttons = if app.git_hover == Some(GitHover::Section(filter)) {
            group_buttons(filter, files.to_vec(), "section", &state, &store)
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
            rows.len(),
        );
        y += rows.len() as f32 * ROW_H;
        for index in visible {
            let row_y = rows_top + index as f32 * ROW_H;
            let row_rect = UiRect::new(rect.left, row_y, content_rect.right, row_y + ROW_H);
            match &rows[index] {
                ChangeRow::Folder { path, files } => {
                    let hover = GitHover::Folder(filter, path.clone());
                    let hovered = app.git_hover.as_ref() == Some(&hover);
                    hover_targets.push((row_rect, hover));
                    content.push(folder_row(
                        row_rect,
                        filter,
                        path,
                        files.clone(),
                        hovered,
                        &state,
                        &store,
                    ));
                }
                &ChangeRow::File { path, nested } => {
                    let file_state = &repository.files[path];
                    let hover = GitHover::File(filter, path.clone());
                    let hovered = app.git_hover.as_ref() == Some(&hover);
                    hover_targets.push((row_rect, hover));
                    let target = if filter == STAGED {
                        DiffTarget::HeadToIndex
                    } else {
                        DiffTarget::IndexToWorktree
                    };
                    let active =
                        app.workspace.active_diff().is_some_and(|diff| {
                            diff.matches(&repository.worktree_root, path, target)
                        }) || app.workspace.active_diff_focus().is_some_and(|focus| {
                            focus.repository_root == repository.worktree_root
                                && focus.path == *path
                                && focus.target == target
                        });
                    content.push(file_row(
                        row_rect,
                        path.clone(),
                        nested,
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
            }
        }
        y += 10.0;
    }
    let changes_bottom = y;
    let changes_content = std::mem::take(&mut content);
    let changes_hover = std::mem::take(&mut hover_targets);
    let changes_rect = content_rect;
    let changes_scroll = scroll;
    let changes_max_scroll = max_scroll;
    let content_rect = details_rect;
    let scroll = app.git_details_scroll.clamp(0.0, details_max_scroll);
    let mut y = content_rect.top;
    if repository.worktrees.len() > 1 {
        let collapsed = app.git_collapsed_sections.contains(&WORKTREES);
        let section_rect = UiRect::new(rect.left, y, content_rect.right, y + SECTION_H);
        hover_targets.push((section_rect, GitHover::Section(WORKTREES)));
        content.push(section_header(
            section_rect,
            WORKTREES,
            "WORKTREES",
            repository.worktrees.len(),
            collapsed,
            Vec::new(),
            state.clone(),
        ));
        y += SECTION_H;
        if !collapsed {
            let rows_top = y;
            let visible = visible_worktree_rows(
                rows_top,
                content_rect.top + scroll,
                content_rect.bottom + scroll,
                repository.worktrees.len(),
            );
            y += repository.worktrees.len() as f32 * ROW_H * 2.0;
            for index in visible {
                let worktree = &repository.worktrees[index];
                let top = rows_top + index as f32 * ROW_H * 2.0;
                let row_rect = UiRect::new(rect.left, top, content_rect.right, top + ROW_H * 2.0);
                let hover = GitHover::Worktree(worktree.path.clone());
                let hovered = app.git_hover.as_ref() == Some(&hover);
                hover_targets.push((row_rect, hover));
                content.push(worktree_row(
                    row_rect,
                    worktree,
                    repository,
                    index == 0,
                    hovered,
                    git.operation.is_none(),
                    state.clone(),
                    store.clone(),
                ));
            }
        }
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
    }
    {
        let collapsed = app.git_collapsed_sections.contains(&HISTORY);
        let section_rect = UiRect::new(rect.left, y, content_rect.right, y + SECTION_H);
        hover_targets.push((section_rect, GitHover::Section(HISTORY)));
        let count = history
            .and_then(|history| history.commits.as_ref().ok())
            .map_or(0, Vec::len);
        content.push(section_header(
            section_rect,
            HISTORY,
            "COMMITS",
            count,
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
                history_rows.len(),
            );
            y += history_rows.len() as f32 * ROW_H;
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_secs() as i64);
            for index in visible {
                let row_y = rows_top + index as f32 * ROW_H;
                let row_rect = UiRect::new(rect.left, row_y, content_rect.right, row_y + ROW_H);
                let row = &history_rows[index];
                let hover = match row {
                    HistoryRow::Commit(commit, _) => Some(GitHover::Commit(commit.oid.clone())),
                    HistoryRow::File { oid, path, .. } => {
                        Some(GitHover::CommitFile(oid.clone(), path.clone()))
                    }
                    HistoryRow::Message(_) => None,
                };
                let hovered = hover.is_some() && app.git_hover == hover;
                if let Some(hover) = hover {
                    hover_targets.push((row_rect, hover));
                }
                content.push(history_row(
                    row_rect,
                    row,
                    hovered,
                    now,
                    &repository.worktree_root,
                    &state,
                    &editor_focus,
                ));
            }
        }
    }
    let details_bottom = y;
    let hover_regions = [
        (changes_rect, changes_scroll, changes_hover),
        (details_rect, scroll, hover_targets),
    ];
    // One hit test per move finds the hovered row, so rows need no hover
    // state of their own and the pointer never leaves a stale highlight.
    let hover_state = state.clone();
    let hover_root = repository.worktree_root.clone();
    root = root.on_event_capture(UiEventKind::PointerMove, move |_cx, payload| {
        if let UiEventPayload::PointerMove { pointer } = payload {
            let point = pointer.point;
            let target = hover_regions
                .iter()
                .find_map(|(viewport, scroll, targets)| {
                    if !viewport.contains(point) {
                        return None;
                    }
                    targets.iter().find_map(|(rect, hover)| {
                        let visible = UiRect::new(
                            rect.left,
                            rect.top - scroll,
                            rect.right,
                            rect.bottom - scroll,
                        );
                        visible.contains(point).then_some((visible, hover))
                    })
                });
            let next = target.as_ref().map(|(_, hover)| (*hover).clone());
            let mut entered = false;
            hover_state.try_update(|app| {
                let changed = app.git_hover != next;
                if changed {
                    entered = true;
                    app.git_commit_card = None;
                }
                app.git_hover = next;
                changed
            });
            if entered && let Some((anchor, GitHover::Commit(oid))) = target {
                git_actions::hover_commit(&hover_state, hover_root.clone(), oid.clone(), anchor);
            }
        }
    });
    root = root
        .child(scroll_region(
            changes_rect,
            changes_content,
            changes_bottom,
            changes_scroll,
            changes_max_scroll,
            state.clone(),
            ScrollRegion::Changes,
        ))
        .child(scroll_region(
            details_rect,
            content,
            details_bottom,
            scroll,
            details_max_scroll,
            state.clone(),
            ScrollRegion::Details,
        ))
        .child(panel(
            UiRect::new(
                rect.left,
                details_rect.top,
                rect.right,
                (details_rect.top + 1.0).min(details_rect.bottom),
            ),
            VisualStyle::filled(theme::c().border),
        ));
    let field = UiRect::new(
        rect.left + 12.0,
        prompt_top + 4.0,
        (rect.right - 12.0).max(rect.left + 12.0),
        (prompt_bottom - 4.0).max(prompt_top + 4.0),
    );
    let button_w = 20.0_f32.min((field.width() - 8.0).max(0.0) / 3.0);
    let options_rect = UiRect::new(
        field.right - 4.0 - button_w,
        field.top + 2.0,
        field.right - 4.0,
        field.bottom - 2.0,
    );
    let commit_button_rect = UiRect::new(
        options_rect.left - 2.0 - button_w,
        options_rect.top,
        options_rect.left - 2.0,
        options_rect.bottom,
    );
    let commit_field = theme::bordered(
        field,
        theme::c().surface,
        if app.git_commit_input.focused {
            theme::c().accent
        } else {
            theme::c().border
        },
        4.0,
        1.0,
    )
    .child(commit_input(
        UiRect::new(
            field.left + 2.0,
            field.top + 1.0,
            (commit_button_rect.left - 2.0).max(field.left + 2.0),
            field.bottom - 1.0,
        ),
        state.clone(),
        commit_focus,
        commit_id,
    ))
    .child(commit_button(
        commit_button_rect,
        state.clone(),
        store.clone(),
    ))
    .child(commit_options_button(options_rect, state.clone()));
    root = root.child(clip(field, 0.0, 0.0).child(commit_field));
    root = root.child(git_resize_handle(rect, state, resizing));
    root
}

fn visible_rows(
    rows_top: f32,
    viewport_top: f32,
    viewport_bottom: f32,
    count: usize,
) -> std::ops::Range<usize> {
    visible_rows_with_height(rows_top, viewport_top, viewport_bottom, count, ROW_H)
}

fn visible_worktree_rows(
    rows_top: f32,
    viewport_top: f32,
    viewport_bottom: f32,
    count: usize,
) -> std::ops::Range<usize> {
    visible_rows_with_height(rows_top, viewport_top, viewport_bottom, count, ROW_H * 2.0)
}

fn visible_rows_with_height(
    rows_top: f32,
    viewport_top: f32,
    viewport_bottom: f32,
    count: usize,
    height: f32,
) -> std::ops::Range<usize> {
    let start = ((viewport_top - rows_top) / height).floor().max(0.0) as usize;
    let end = ((viewport_bottom - rows_top) / height).ceil().max(0.0) as usize;
    start.saturating_sub(1).min(count)..end.saturating_add(1).min(count)
}

#[cfg(test)]
mod virtualization_tests {
    use super::*;

    #[test]
    fn bottom_sections_stay_pinned_and_scroll_independently_from_changes() {
        use crate::git::RepositoryId;
        use crate::git::store::{GitPresentation, RepositoryPresentation};
        use lgui::application::{AppView, ApplicationContext};
        use lgui::core::{
            InputEvent, Point, PointerData, UiScale, WheelDelta, dispatch_runtime_output,
        };
        use lgui::session::UiSession;
        use std::sync::{Arc, Mutex};

        let viewport = UiRect::new(0.0, 0.0, 340.0, 600.0);
        let exposed = Arc::new(Mutex::new(
            None::<(State<AppState>, State<GitStoreSnapshot>)>,
        ));
        let output = exposed.clone();
        let id = RepositoryId(1);
        let paths = (0..10_000)
            .map(|index| PathBuf::from(format!("src/file{index:05}.rs")))
            .collect::<Vec<_>>();
        let repository = RepositorySnapshot {
            id,
            worktree_root: PathBuf::from("loom"),
            git_dir: PathBuf::from("loom/.git"),
            common_dir: PathBuf::from("loom/.git"),
            head: crate::git::HeadState::Branch("main".into()),
            upstream: None,
            ahead: 0,
            behind: 0,
            files: Arc::new(
                paths
                    .iter()
                    .cloned()
                    .map(|path| {
                        (
                            path,
                            FileState {
                                worktree: ChangeKind::Modified,
                                ..Default::default()
                            },
                        )
                    })
                    .collect(),
            ),
            ignored: Default::default(),
            repository_state: RepositoryState::Normal,
            operation_step: None,
            head_oid: None,
            stashes: Arc::new(vec![StashInfo {
                index: 0,
                oid: "stash".into(),
                message: "WIP: search panel".into(),
            }]),
            worktrees: Arc::new(
                [("loom", "main"), ("loom-search", "feature/search")]
                    .into_iter()
                    .map(|(path, branch)| WorktreeInfo {
                        path: path.into(),
                        oid: "abc".into(),
                        branch: Some(branch.into()),
                        bare: false,
                        detached: false,
                        locked: None,
                    })
                    .collect(),
            ),
            features: Default::default(),
            generation: 0,
        };
        let mut prepared = RepositoryPresentation::default();
        prepared.sections[UNSTAGED as usize] = paths;
        let snapshot = GitStoreSnapshot {
            repositories: Arc::new([(id, repository)].into_iter().collect()),
            active_repository: Some(id),
            presentation: Arc::new(GitPresentation {
                repositories: [(id, prepared)].into_iter().collect(),
                ..Default::default()
            }),
            ..Default::default()
        };
        let view: AppView = Arc::new(move |cx| {
            let state = cx.state_with(|| {
                let mut app = AppState::new();
                app.git_history = Some(GitHistory {
                    repository: "loom".into(),
                    head: None,
                    commits: Ok((0..40)
                        .map(|index| CommitSummary {
                            oid: format!("abc{index:05}"),
                            parents: Vec::new(),
                            author: "lius-new".into(),
                            author_email: "dev@example.com".into(),
                            timestamp: 0,
                            subject: format!("Improve editor behavior {index}"),
                            decorations: Vec::new(),
                        })
                        .collect()),
                });
                app
            });
            let fixture = snapshot.clone();
            let store = cx.state_with(move || fixture);
            *output.lock().unwrap() = Some((state.clone(), store.clone()));
            render(
                viewport,
                state,
                store,
                cx.focus_handle(UiId::new("git-test-editor")),
                cx.focus_handle(UiId::new("git-test-commit")),
                UiId::new("git-test-commit"),
            )
        });
        let context = ApplicationContext::empty(Default::default());
        context
            .resources()
            .provide(lgui::icons::IconRegistration(Arc::new(crate::svg_icons())));
        let _fonts = lgui::backend::install_text_environment(
            &context,
            lgui::render_skia::skia_text_system_handle(),
        );
        let mut session = UiSession::new();
        session.render_view(&view, viewport, UiScale::ONE);
        let (state, store) = exposed.lock().unwrap().clone().unwrap();
        let find = |session: &UiSession, name: &str| {
            session
                .tree()
                .nodes()
                .iter()
                .find(|node| {
                    node.semantics
                        .as_ref()
                        .and_then(|semantics| semantics.name.as_deref())
                        == Some(name)
                })
                .unwrap_or_else(|| panic!("Missing section {name}"))
                .hit_rect
        };
        let footer_top = viewport.bottom;
        let worktrees = find(&session, "WORKTREES");
        let stashes = find(&session, "STASHES");
        let commits = find(&session, "COMMITS");
        assert_eq!(worktrees.top, footer_top - SECTION_H * 3.0);
        assert_eq!(stashes.top, worktrees.bottom);
        assert_eq!(commits.bottom, footer_top);
        assert!(find(&session, "UNSTAGED").top < worktrees.top);
        let commit_input = find(&session, "Commit message");
        assert_eq!(commit_input.bottom + 5.0, worktrees.top);
        assert_eq!(commit_input.height(), 22.0);
        save_drawer_preview(&session, &context, viewport, "git-drawer-long-changes");
        assert!(
            session.tree().nodes().len() < 600,
            "Large lists must stay virtualized"
        );
        let send = |session: &mut UiSession, input| {
            dispatch_runtime_output(
                session.handle_input(input),
                &context,
                &lgui::window::WindowId::new("git-drawer-test"),
                |action| session.handle_default_action(action),
                |_| {},
            );
        };
        send(
            &mut session,
            InputEvent::Wheel {
                point: Point::new(80.0, 150.0),
                delta: WheelDelta::lines(0.0, -4.0),
            },
        );
        session.render_view(&view, viewport, UiScale::ONE);
        assert!(state.get().git_scroll > 0.0);
        assert_eq!(state.get().git_details_scroll, 0.0);
        assert_eq!(find(&session, "WORKTREES"), worktrees);
        assert_eq!(find(&session, "COMMITS"), commits);
        assert_eq!(find(&session, "Commit message"), commit_input);
        let changes_scroll = state.get().git_scroll;
        let point = Point::new(100.0, commits.top + SECTION_H / 2.0);
        for input in [
            InputEvent::PointerDown {
                pointer: PointerData::mouse(point),
                button: PointerButton::Left,
            },
            InputEvent::PointerUp {
                pointer: PointerData::mouse(point),
                button: PointerButton::Left,
            },
        ] {
            send(&mut session, input);
        }
        session.render_view(&view, viewport, UiScale::ONE);
        assert!(!state.get().git_collapsed_sections.contains(&HISTORY));
        let expanded = find(&session, "COMMITS");
        assert!(expanded.top >= TOPLINE_H + (footer_top - TOPLINE_H) * 0.55);
        send(
            &mut session,
            InputEvent::Wheel {
                point: Point::new(100.0, expanded.bottom + ROW_H),
                delta: WheelDelta::lines(0.0, -3.0),
            },
        );
        session.render_view(&view, viewport, UiScale::ONE);
        assert!(state.get().git_details_scroll > 0.0);
        assert_eq!(state.get().git_scroll, changes_scroll);

        // Visual fixtures use a short list to expose the empty space between
        // the primary changes and the sections pinned at the bottom.
        store.update(|snapshot| {
            let presentation = Arc::make_mut(&mut snapshot.presentation);
            presentation.repositories.get_mut(&id).unwrap().sections[UNSTAGED as usize].truncate(5);
        });
        state.update(|app| {
            app.git_scroll = 0.0;
            app.git_details_scroll = 0.0;
            app.git_collapsed_sections.insert(HISTORY);
        });
        session.render_view(&view, viewport, UiScale::ONE);
        let short_input = find(&session, "Commit message");
        assert_eq!(
            short_input.top,
            TOPLINE_H + 8.0 + SECTION_H + 5.0 * ROW_H + 10.0 + 5.0
        );
        assert!(short_input.bottom < find(&session, "WORKTREES").top);
        let point = Point::new(short_input.left + 30.0, short_input.top + PROMPT_H / 2.0);
        for input in [
            InputEvent::PointerDown {
                pointer: PointerData::mouse(point),
                button: PointerButton::Left,
            },
            InputEvent::PointerUp {
                pointer: PointerData::mouse(point),
                button: PointerButton::Left,
            },
            InputEvent::TextInput("Improve search UI".into()),
        ] {
            send(&mut session, input);
        }
        session.render_view(&view, viewport, UiScale::ONE);
        assert_eq!(state.get().git_commit_input.text(), "Improve search UI");
        save_drawer_preview(&session, &context, viewport, "git-drawer-collapsed");
        state.update(|app| {
            app.git_collapsed_sections.remove(&HISTORY);
        });
        session.render_view(&view, viewport, UiScale::ONE);
        save_drawer_preview(&session, &context, viewport, "git-drawer-commits");
        state.update(|app| {
            app.git_collapsed_sections.insert(HISTORY);
            app.git_collapsed_sections.remove(&WORKTREES);
        });
        session.render_view(&view, viewport, UiScale::ONE);
        save_drawer_preview(&session, &context, viewport, "git-drawer-worktrees");
        store.update(|snapshot| {
            Arc::make_mut(&mut snapshot.presentation)
                .repositories
                .get_mut(&id)
                .unwrap()
                .sections[UNSTAGED as usize]
                .clear();
        });
        state.update(|app| {
            app.git_commit_input.clear();
            app.git_commit_input.focused = false;
            app.git_collapsed_sections.insert(WORKTREES);
        });
        session.render_view(&view, viewport, UiScale::ONE);
        assert_eq!(find(&session, "Commit message").top, TOPLINE_H + 8.0 + 5.0);
        save_drawer_preview(&session, &context, viewport, "git-drawer-no-changes");
        for height in [120.0, 300.0, 900.0] {
            let body = UiRect::new(0.0, TOPLINE_H, 340.0, height);
            let (changes, prompt, details) = drawer_regions(body, 5000.0, 1500.0, 3);
            assert_eq!(changes.bottom, prompt.top);
            assert_eq!(prompt.bottom, details.top);
            assert_eq!(prompt.height(), PROMPT_H);
            assert_eq!(details.bottom, body.bottom);
            assert!(changes.height() >= 0.0 && details.height() >= 0.0);
        }
        state.update(|app| {
            app.git_scroll = 100.0;
            app.git_details_scroll = 200.0;
        });
        git_actions::select_repository(&state, &store, PathBuf::from("loom"));
        assert_eq!(state.get().git_scroll, 0.0);
        assert_eq!(state.get().git_details_scroll, 0.0);
    }

    fn save_drawer_preview(
        session: &lgui::session::UiSession,
        context: &lgui::application::ApplicationContext,
        rect: UiRect,
        name: &str,
    ) {
        let Some(directory) = std::env::var_os("LOOM_GIT_PREVIEW_DIR") else {
            return;
        };
        use lgui::core::{PhysicalRect, UiScale};
        use lgui::renderer::{FrameInfo, FrameReason};
        let physical = PhysicalRect::new(0, 0, rect.width() as i32, rect.height() as i32);
        let dirty = [physical];
        let frame = FrameInfo::new(
            physical,
            &dirty,
            UiScale::ONE,
            FrameReason::SceneChange,
            true,
        );
        let mut surface = lgui::render_skia::SkiaSoftwareSurface::new(16 * 1024 * 1024);
        lgui::assets::backend::with_render_resources(
            context,
            lgui::assets::backend::render_resources(),
            || {
                surface.draw(
                    &session.tree().scene().project_to_physical(UiScale::ONE),
                    &frame,
                )
            },
        )
        .unwrap();
        let mut rgba = surface.pixels().to_vec();
        for pixel in rgba.as_chunks_mut::<4>().0 {
            pixel.swap(0, 2);
        }
        image::save_buffer(
            std::path::Path::new(&directory).join(format!("{name}.png")),
            &rgba,
            rect.width() as u32,
            rect.height() as u32,
            image::ColorType::Rgba8,
        )
        .unwrap();
    }

    #[test]
    fn worktree_delete_button_only_requests_confirmation() {
        use crate::ui::git_confirm::GitConfirmAction;
        use lgui::application::{AppView, ApplicationContext};
        use lgui::core::{InputEvent, Point, PointerData, UiScale, dispatch_runtime_output};
        use lgui::session::UiSession;
        use std::sync::{Arc, Mutex};

        let exposed = Arc::new(Mutex::new(None::<State<AppState>>));
        let output = exposed.clone();
        let viewport = UiRect::new(0.0, 0.0, 300.0, 100.0);
        let source = PathBuf::from("worktree-ui-test-source");
        let target = PathBuf::from("worktree-ui-test-linked");
        let repository = RepositorySnapshot {
            id: crate::git::RepositoryId(1),
            worktree_root: source.clone(),
            git_dir: source.join(".git"),
            common_dir: source.join(".git"),
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
            worktrees: Default::default(),
            features: Default::default(),
            generation: 0,
        };
        let worktree = WorktreeInfo {
            path: target.clone(),
            oid: "abc".into(),
            branch: Some("feature/x".into()),
            bare: false,
            detached: false,
            locked: None,
        };
        let view: AppView = Arc::new(move |cx| {
            let source = repository.worktree_root.clone();
            let state = cx.state_with(move || {
                let mut app = AppState::new();
                app.workspace_folders = vec![source];
                app
            });
            let store = cx.state_with(GitStoreSnapshot::default);
            *output.lock().unwrap() = Some(state.clone());
            panel(viewport, VisualStyle::default()).child(worktree_row(
                UiRect::new(0.0, 0.0, 300.0, ROW_H * 2.0),
                &worktree,
                &repository,
                false,
                true,
                true,
                state,
                store,
            ))
        });
        let mut session = UiSession::new();
        session.render_view(&view, viewport, UiScale::ONE);
        let state = exposed.lock().unwrap().clone().unwrap();
        let context = ApplicationContext::empty(Default::default());
        let point = Point::new(282.0, ROW_H);
        for input in [
            InputEvent::PointerDown {
                pointer: PointerData::mouse(point),
                button: PointerButton::Left,
            },
            InputEvent::PointerUp {
                pointer: PointerData::mouse(point),
                button: PointerButton::Left,
            },
        ] {
            let events = session.handle_input(input);
            dispatch_runtime_output(
                events,
                &context,
                &lgui::window::WindowId::new("worktree-actions-test"),
                |action| session.handle_default_action(action),
                |_| {},
            );
            session.render_view(&view, viewport, UiScale::ONE);
        }
        let app = state.get();
        assert_eq!(app.workspace_folders, [source.clone()]);
        assert!(app.git_pinned_repository.is_none());
        assert!(app.toast.is_none()); // The click did not also open the row.
        let confirm = app.git_confirm.unwrap();
        assert!(confirm.destructive);
        assert_eq!(
            confirm.action,
            GitConfirmAction::RemoveWorktree {
                repository: source,
                path: target,
                force: false
            }
        );
    }

    #[test]
    fn scrollbar_is_separate_from_resize_handle_and_drags_outside_its_strip() {
        use lgui::application::{AppView, ApplicationContext};
        use lgui::core::{InputEvent, Point, PointerData, UiScale, dispatch_runtime_output};
        use lgui::session::UiSession;
        use std::sync::{Arc, Mutex};

        for region in [ScrollRegion::Changes, ScrollRegion::Details] {
            let exposed = Arc::new(Mutex::new(None::<State<AppState>>));
            let output = exposed.clone();
            let viewport = UiRect::new(0.0, 0.0, 300.0, 300.0);
            let view: AppView = Arc::new(move |cx| {
                let state = cx.state_with(AppState::new);
                *output.lock().unwrap() = Some(state.clone());
                let scroll = region.scroll(&state.get());
                panel(viewport, VisualStyle::default())
                    .child(git_vertical_scrollbar(
                        viewport,
                        1200.0,
                        scroll,
                        state.clone(),
                        region,
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
            assert!(region.drag(&state.get()).0);
            assert!(!state.get().resizing_git_sidebar);
            let outside = PointerData::mouse(Point::new(30.0, 260.0));
            send(InputEvent::PointerMove(outside));
            assert!(region.scroll(&state.get()) > 0.0);
            let other = match region {
                ScrollRegion::Changes => ScrollRegion::Details,
                ScrollRegion::Details => ScrollRegion::Changes,
            };
            assert_eq!(other.scroll(&state.get()), 0.0);
            send(InputEvent::PointerUp {
                pointer: outside,
                button: PointerButton::Left,
            });
            assert!(!region.drag(&state.get()).0);
            let released = region.scroll(&state.get());
            send(InputEvent::PointerMove(PointerData::mouse(point)));
            assert_eq!(region.scroll(&state.get()), released);
        }
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
    region: ScrollRegion,
) -> Element {
    let content_h = (content_bottom - rect.top).max(1.0);
    let viewport_h = rect.height();
    let max_scroll = (content_bottom - rect.bottom).max(0.0);
    let track = UiRect::new(
        rect.right - 8.0 - scrollbar::SIZE,
        rect.top + 8.0,
        rect.right - 8.0,
        rect.bottom - 8.0,
    );
    if track.height() <= 0.0 {
        return lgui::prelude::group(rect);
    }
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
    .key(format!("{}-scrollbar-thumb", region.key()))
    .event_policy(EventPolicy::INTERACTIVE)
    .on_pointer_down(move |_cx, pointer| {
        drag_start.update(move |app| {
            region.set_drag(app, true, pointer.point.y - thumb_top);
        });
    })
    .on_pointer_drag(move |_cx, pointer| {
        drag_move.update(move |app| {
            let (dragging, offset) = region.drag(app);
            if dragging && travel > 0.0 {
                let next = (pointer.point.y - track_top - offset) / travel * max_scroll;
                region.set_scroll(app, next.clamp(0.0, max_scroll));
            }
        });
    })
    .on_pointer_up(move |_cx, _pointer| {
        drag_end.update(|app| region.set_drag(app, false, 0.0));
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
        "Commit message"
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
                placeholder: theme::mono(theme::c().text_muted, theme::SMALL),
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
    panel(rect, VisualStyle::default())
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

/// Change-section indices in `RepositoryPresentation::sections`, plus stable
/// identifiers used by `git_collapsed_sections` for the bottom detail sections.
pub const CONFLICTS: u8 = 0;
pub const STAGED: u8 = 1;
pub const UNSTAGED: u8 = 2;
pub const UNTRACKED: u8 = 3;
/// Listed after the change sections; its rows come from the snapshot.
pub const STASHES: u8 = 4;
/// The recent commits, last in the drawer.
pub const HISTORY: u8 = 5;
pub const WORKTREES: u8 = 6;

/// The drawer row under the pointer, which shows its action buttons.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitHover {
    Worktree(PathBuf),
    Section(u8),
    File(u8, PathBuf),
    /// A folder row of a section grouped by folder.
    Folder(u8, PathBuf),
    /// A stash, by commit id.
    Stash(String),
    /// A history commit, by commit id.
    Commit(String),
    /// A file of an expanded history commit.
    CommitFile(String, PathBuf),
}

/// Recent commits of a repository's HEAD.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHistory {
    pub repository: PathBuf,
    /// The commit HEAD was at when these were listed.
    pub head: Option<String>,
    pub commits: Result<Vec<CommitSummary>, String>,
}

/// A history commit's hover card, beside its row.
#[derive(Clone, Debug, PartialEq)]
pub struct CommitCard {
    pub oid: String,
    /// The commit's row on screen when the card was requested.
    pub anchor: UiRect,
    pub details: Result<crate::git::CommitDetails, String>,
}

const CARD_W: f32 = 420.0;
const CARD_PAD: f32 = 12.0;
const CARD_LINE_H: f32 = 18.0;
const CARD_MAX_MESSAGE_LINES: usize = 16;

/// The hovered history commit's card, to the right of Source Control.
pub fn render_commit_card(viewport: UiRect, app: &AppState) -> Option<Element> {
    let card = app.git_commit_card.as_ref()?;
    if app.git_hover.as_ref() != Some(&GitHover::Commit(card.oid.clone())) {
        return None;
    }
    let summary = app
        .git_history
        .as_ref()
        .and_then(|history| history.commits.as_ref().ok())
        .and_then(|commits| commits.iter().find(|commit| commit.oid == card.oid));
    let width = CARD_W.min(viewport.width() - 16.0).max(160.0);
    let inner = width - CARD_PAD * 2.0;
    let separator = 11.0;

    // Each line is (text pieces with colors, bold).
    enum Line {
        Text(Vec<(String, lgui::prelude::Color)>, bool),
        Separator,
    }
    let mut lines = Vec::new();
    match &card.details {
        Ok(details) => {
            lines.push(Line::Text(
                vec![(details.author.clone(), theme::c().text_bright)],
                true,
            ));
            lines.push(Line::Text(
                vec![(
                    format!("{}  ·  {}", details.author_email, details.date),
                    theme::c().text_dim,
                )],
                false,
            ));
            lines.push(Line::Separator);
            let mut message = wrap_text(&details.message.replace('\t', "    "), inner);
            if message.len() > CARD_MAX_MESSAGE_LINES {
                message.truncate(CARD_MAX_MESSAGE_LINES);
                message.push("…".into());
            }
            lines.extend(
                message
                    .into_iter()
                    .map(|line| Line::Text(vec![(line, theme::c().text)], false)),
            );
            if details.files_changed > 0 {
                lines.push(Line::Separator);
                let plural = |count: usize, word: &str| {
                    format!("{count} {word}{}", if count == 1 { "" } else { "s" })
                };
                let mut stats = vec![(
                    format!("{} changed", plural(details.files_changed, "file")),
                    theme::c().text_soft,
                )];
                if details.insertions > 0 {
                    stats.push((", ".into(), theme::c().text_soft));
                    stats.push((
                        format!("{}(+)", plural(details.insertions, "insertion")),
                        theme::c().git.added,
                    ));
                }
                if details.deletions > 0 {
                    stats.push((", ".into(), theme::c().text_soft));
                    stats.push((
                        format!("{}(-)", plural(details.deletions, "deletion")),
                        theme::c().git.deleted,
                    ));
                }
                lines.push(Line::Text(stats, false));
            }
        }
        Err(error) => lines.extend(
            wrap_text(error, inner)
                .into_iter()
                .map(|line| Line::Text(vec![(line, theme::c().error_text)], false)),
        ),
    }
    lines.push(Line::Separator);
    let mut footer = vec![(
        crate::model::diff_document::short_oid(&card.oid).to_owned(),
        theme::c().text_soft,
    )];
    if let Some(summary) = summary.filter(|summary| !summary.decorations.is_empty()) {
        footer.push((
            format!("  {}", summary.decorations.join(", ")),
            theme::c().text_dim,
        ));
    }
    lines.push(Line::Text(footer, false));

    let height = CARD_PAD * 2.0
        + lines
            .iter()
            .map(|line| match line {
                Line::Text(..) => CARD_LINE_H,
                Line::Separator => separator,
            })
            .sum::<f32>();
    let left = (card.anchor.right + 6.0)
        .min(viewport.right - width - 8.0)
        .max(viewport.left + 8.0);
    let top = card
        .anchor
        .top
        .min(viewport.bottom - height - 8.0)
        .max(viewport.top + 8.0);
    let rect = UiRect::new(left, top, left + width, top + height);
    let mut surface = theme::bordered(rect, theme::c().surface, theme::c().border, 6.0, 1.0)
        .shadow(
            lgui::prelude::ShadowStyle::new(lgui::prelude::Color::BLACK)
                .alpha(theme::c().shadow_alpha)
                .offset(0.0, 2.0)
                .blur(3.0),
        );
    let mut y = rect.top + CARD_PAD;
    for line in lines {
        match line {
            Line::Separator => {
                let line_y = y + (separator - 1.0) / 2.0;
                surface = surface.child(panel(
                    UiRect::new(
                        rect.left + CARD_PAD,
                        line_y,
                        rect.right - CARD_PAD,
                        line_y + 1.0,
                    ),
                    VisualStyle::filled(theme::c().border),
                ));
                y += separator;
            }
            Line::Text(pieces, bold) => {
                let mut x = rect.left + CARD_PAD;
                for (piece, color) in pieces {
                    let piece_w =
                        super::tabs::measure(&piece, theme::UI_SIZE, if bold { 700 } else { 400 });
                    let piece = super::tabs::ellipsize(
                        &piece,
                        (rect.right - CARD_PAD - x).max(0.0),
                        theme::UI_SIZE,
                        if bold { 700 } else { 400 },
                    );
                    surface = surface.child(text(
                        UiRect::new(x, y, rect.right - CARD_PAD, y + CARD_LINE_H),
                        piece,
                        if bold {
                            theme::mono_bold(color, theme::UI_SIZE)
                        } else {
                            theme::mono(color, theme::UI_SIZE)
                        },
                    ));
                    x += piece_w;
                }
                y += CARD_LINE_H;
            }
        }
    }
    Some(surface)
}

/// Break `text` into lines no wider than `width`: at spaces where possible,
/// otherwise between characters, as CJK text needs.
fn wrap_text(text: &str, width: f32) -> Vec<String> {
    let fits = |line: &str| super::tabs::measure(line, theme::UI_SIZE, 400) <= width;
    let mut lines = Vec::new();
    for paragraph in text.lines() {
        let mut line = String::new();
        for word in paragraph.split_inclusive(' ') {
            if fits(format!("{line}{word}").trim_end()) {
                line.push_str(word);
                continue;
            }
            if !line.is_empty() {
                lines.push(std::mem::take(&mut line).trim_end().to_owned());
            }
            for ch in word.chars() {
                line.push(ch);
                if !fits(&line) && line.chars().count() > 1 {
                    line.pop();
                    lines.push(std::mem::take(&mut line));
                    line.push(ch);
                }
            }
        }
        lines.push(line.trim_end().to_owned());
    }
    lines
}

/// A history commit expanded to list the files it changed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExpandedCommit {
    pub oid: String,
    /// `None` while loading.
    pub files: Option<Result<Vec<(ChangeKind, PathBuf)>, String>>,
}

const BANNER_H: f32 = 30.0;

/// The folder name shown for a repository.
fn repository_name(root: &std::path::Path) -> String {
    root.file_name().map_or_else(
        || super::display_path(root),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// A row of a change section.
#[derive(Clone, Debug, PartialEq, Eq)]
enum ChangeRow<'a> {
    /// A folder and the section's files directly in it.
    Folder { path: PathBuf, files: Vec<PathBuf> },
    /// A file; `nested` when listed under its folder row.
    File { path: &'a PathBuf, nested: bool },
}

/// `files` as rows: as given, or grouped under one row per folder (files
/// at the repository root first, without a folder row).
fn change_rows(files: &[PathBuf], grouped: bool) -> Vec<ChangeRow<'_>> {
    if !grouped {
        return files
            .iter()
            .map(|path| ChangeRow::File {
                path,
                nested: false,
            })
            .collect();
    }
    let mut folders = std::collections::BTreeMap::<PathBuf, Vec<&PathBuf>>::new();
    let mut rows = Vec::new();
    for path in files {
        match path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            Some(parent) => folders.entry(parent.to_path_buf()).or_default().push(path),
            None => rows.push(ChangeRow::File {
                path,
                nested: false,
            }),
        }
    }
    for (folder, paths) in folders {
        rows.push(ChangeRow::Folder {
            path: folder,
            files: paths.iter().map(|path| (*path).clone()).collect(),
        });
        rows.extend(
            paths
                .into_iter()
                .map(|path| ChangeRow::File { path, nested: true }),
        );
    }
    rows
}

/// A section's bulk actions for `paths`, as buttons.
fn group_buttons(
    filter: u8,
    paths: Vec<PathBuf>,
    key: &str,
    state: &State<AppState>,
    store: &State<GitStoreSnapshot>,
) -> Vec<RowButton> {
    section_actions(filter)
        .iter()
        .map(|&action| {
            let action_state = state.clone();
            let action_store = store.clone();
            let paths = paths.clone();
            RowButton {
                key: format!("git-{key}-{filter}-{action:?}"),
                icon: action.icon(),
                name: action.section_name(filter),
                on_click: Box::new(move || {
                    action.run(&action_state, &action_store, filter, paths.clone());
                }),
            }
        })
        .collect()
}

/// A folder of a grouped section, with the section's actions for its files.
fn folder_row(
    rect: UiRect,
    filter: u8,
    path: &std::path::Path,
    files: Vec<PathBuf>,
    hovered: bool,
    state: &State<AppState>,
    store: &State<GitStoreSnapshot>,
) -> Element {
    let label = path.display().to_string();
    let key = format!("folder-{}", label);
    let mut row = panel(
        rect,
        if hovered {
            VisualStyle::filled(theme::c().surface)
        } else {
            VisualStyle::default()
        },
    )
    .key(format!("git-{key}-{filter}"))
    .child(super::sidebar::drawer_icon(
        "git-folder-row-icon",
        "folder",
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
            UiRect::new(rect.left + 30.0, rect.top, rect.right - 52.0, rect.bottom),
            0.0,
            0.0,
        )
        .child(text(
            UiRect::new(rect.left + 30.0, rect.top, rect.left + 2000.0, rect.bottom),
            label,
            theme::mono(theme::c().text_muted, theme::SMALL),
        )),
    );
    if hovered {
        let buttons = group_buttons(filter, files, &key, state, store);
        row = row_buttons(row, rect, rect.right - 8.0, buttons);
    }
    row
}

/// `path`'s file name, then its folder dimmed when `show_folder`, clipped
/// to `viewport`. The name leads so a deep path still shows which file it
/// is; a long folder is cut at the right.
fn path_label(
    viewport: UiRect,
    path: &std::path::Path,
    show_folder: bool,
    emphasized: bool,
) -> Element {
    let (name, folder) = split_path(path);
    let name_style = theme::mono(
        if emphasized {
            theme::c().text
        } else {
            theme::c().text_soft
        },
        theme::SMALL,
    );
    let folder_style = theme::mono(theme::c().text_ghost, theme::SMALL);
    // Lay each text out at its natural width and let the viewport clip it.
    // Narrower rects make the text engine wrap at path separators instead.
    let name_w = super::tabs::measure(&name, theme::SMALL, 400) + TEXT_PAD;
    let mut label = clip(viewport, 0.0, 0.0).child(text(
        UiRect::new(
            viewport.left,
            viewport.top,
            viewport.left + name_w,
            viewport.bottom,
        ),
        name,
        name_style,
    ));
    if let Some(folder) = folder.filter(|_| show_folder) {
        let left = viewport.left + name_w + 4.0;
        let folder_w = super::tabs::measure(&folder, theme::SMALL, 400) + TEXT_PAD;
        label = label.child(text(
            UiRect::new(left, viewport.top, left + folder_w, viewport.bottom),
            folder,
            folder_style,
        ));
    }
    label
}

/// Extra width past a measured text so its last glyph is never clipped.
const TEXT_PAD: f32 = 6.0;

/// `path` as its file name and its folder, `None` at the repository root.
fn split_path(path: &std::path::Path) -> (String, Option<String>) {
    let name = path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    );
    let folder = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(|parent| parent.display().to_string());
    (name, folder)
}

/// A row of the history section.
#[derive(Clone, Debug, PartialEq, Eq)]
enum HistoryRow {
    /// A commit, and whether it is expanded.
    Commit(CommitSummary, bool),
    /// A file changed by the expanded commit `oid`.
    File {
        oid: String,
        kind: ChangeKind,
        path: PathBuf,
    },
    /// Loading, empty or failure text.
    Message(String),
}

fn history_rows(
    history: Option<&GitHistory>,
    expanded: Option<&ExpandedCommit>,
) -> Vec<HistoryRow> {
    let commits = match history.map(|history| &history.commits) {
        None => return vec![HistoryRow::Message("Loading commits…".into())],
        Some(Err(error)) => return vec![HistoryRow::Message(error.clone())],
        Some(Ok(commits)) if commits.is_empty() => {
            return vec![HistoryRow::Message("No commits yet.".into())];
        }
        Some(Ok(commits)) => commits,
    };
    let mut rows = Vec::new();
    for commit in commits {
        let open = expanded.filter(|expanded| expanded.oid == commit.oid);
        rows.push(HistoryRow::Commit(commit.clone(), open.is_some()));
        let Some(open) = open else {
            continue;
        };
        match &open.files {
            None => rows.push(HistoryRow::Message("Loading files…".into())),
            Some(Err(error)) => rows.push(HistoryRow::Message(error.clone())),
            Some(Ok(files)) => rows.extend(files.iter().map(|(kind, path)| HistoryRow::File {
                oid: commit.oid.clone(),
                kind: *kind,
                path: path.clone(),
            })),
        }
    }
    rows
}

/// `timestamp` (Unix seconds) as a short age relative to `now`.
fn relative_time(now: i64, timestamp: i64) -> String {
    let seconds = (now - timestamp).max(0);
    match seconds {
        0..60 => "now".into(),
        60..3_600 => format!("{}m", seconds / 60),
        3_600..86_400 => format!("{}h", seconds / 3_600),
        86_400..604_800 => format!("{}d", seconds / 86_400),
        604_800..2_592_000 => format!("{}w", seconds / 604_800),
        2_592_000..31_536_000 => format!("{}mo", seconds / 2_592_000),
        _ => format!("{}y", seconds / 31_536_000),
    }
}

fn history_row(
    rect: UiRect,
    row: &HistoryRow,
    hovered: bool,
    now: i64,
    repository: &std::path::Path,
    state: &State<AppState>,
    editor_focus: &UiFocusHandle,
) -> Element {
    let background = if hovered {
        VisualStyle::filled(theme::c().surface)
    } else {
        VisualStyle::default()
    };
    match row {
        HistoryRow::Message(message) => panel(rect, VisualStyle::default()).child(text(
            UiRect::new(rect.left + 30.0, rect.top, rect.right - 12.0, rect.bottom),
            message.clone(),
            theme::mono(theme::c().text_ghost, theme::SMALL),
        )),
        HistoryRow::Commit(commit, open) => {
            let toggle_state = state.clone();
            let root = repository.to_path_buf();
            let oid = commit.oid.clone();
            let menu_state = state.clone();
            let menu_root = repository.to_path_buf();
            let menu_oid = commit.oid.clone();
            let age = relative_time(now, commit.timestamp);
            let age_w = super::tabs::measure(&age, 9.0, 400) + 6.0;
            panel(rect, background)
                .key(format!("git-commit-{}", commit.oid))
                .event_policy(EventPolicy::INTERACTIVE)
                .semantics(Semantics::new(SemanticRole::Button).name(commit.subject.clone()))
                .cursor(CursorIcon::Pointer)
                .on_click(move || {
                    // A right-click that opened this row's menu is not a click.
                    let mut menu_open = false;
                    toggle_state.try_update(|app| {
                        menu_open = app.git_menu.is_some();
                        false
                    });
                    if !menu_open {
                        git_actions::toggle_commit(&toggle_state, root.clone(), oid.clone());
                    }
                })
                .on_pointer_down_with_button(move |_cx, pointer, button| {
                    if button == PointerButton::Right {
                        let target = GitMenuTarget::HistoryCommit {
                            repository: menu_root.clone(),
                            oid: menu_oid.clone(),
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
                .child(super::sidebar::drawer_icon(
                    lgui::core::UiId::owned(format!("git-commit-row-chevron-{}", commit.oid)),
                    if *open {
                        "chevron-down"
                    } else {
                        "chevron-right"
                    },
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
                            rect.right - 14.0 - age_w,
                            rect.bottom,
                        ),
                        0.0,
                        0.0,
                    )
                    .child(text(
                        UiRect::new(rect.left + 30.0, rect.top, rect.left + 2000.0, rect.bottom),
                        commit.subject.clone(),
                        theme::mono(
                            if hovered {
                                theme::c().text
                            } else {
                                theme::c().text_dim
                            },
                            theme::SMALL,
                        ),
                    )),
                )
                .child(text(
                    UiRect::new(
                        rect.right - 12.0 - age_w,
                        rect.top,
                        rect.right - 12.0,
                        rect.bottom,
                    ),
                    age,
                    theme::mono_right(theme::c().text_ghost, 9.0),
                ))
        }
        HistoryRow::File { oid, kind, path } => {
            let open_state = state.clone();
            let open_focus = editor_focus.clone();
            let root = repository.to_path_buf();
            let open_oid = oid.clone();
            let open_path = path.clone();
            let color = match kind {
                ChangeKind::Added | ChangeKind::Copied => theme::c().git.added,
                ChangeKind::Deleted => theme::c().git.deleted,
                _ => theme::c().git.modified,
            };
            panel(rect, background)
                .key(format!("git-commit-{oid}-{}", path.display()))
                .event_policy(EventPolicy::INTERACTIVE)
                .cursor(CursorIcon::Pointer)
                .on_click(move || {
                    git_actions::open_commit_changes(
                        &open_state,
                        root.clone(),
                        open_oid.clone(),
                        Some(open_path.clone()),
                    );
                    open_focus.focus();
                })
                .child(text(
                    UiRect::new(rect.left + 30.0, rect.top, rect.left + 42.0, rect.bottom),
                    kind.indicator(),
                    theme::mono_bold(color, theme::SMALL),
                ))
                .child(path_label(
                    UiRect::new(rect.left + 44.0, rect.top, rect.right - 12.0, rect.bottom),
                    path,
                    true,
                    hovered,
                ))
        }
    }
}

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
        STASHES => "git-section-stashes-chevron",
        WORKTREES => "git-section-worktrees-chevron",
        _ => "git-section-history-chevron",
    };
    let section = panel(rect, VisualStyle::default())
        .key(format!("git-section-{filter}"))
        .semantics({
            let mut semantics = Semantics::new(SemanticRole::Button).name(title);
            semantics.state.expanded = Some(!collapsed);
            semantics
        })
        .event_policy(EventPolicy::INTERACTIVE)
        .cursor(CursorIcon::Pointer)
        .on_click(move || {
            state.update(move |app| {
                if !app.git_collapsed_sections.insert(filter) {
                    app.git_collapsed_sections.remove(&filter);
                }
                if filter >= STASHES {
                    app.git_details_scroll = 0.0;
                    app.git_commit_card = None;
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

#[allow(clippy::too_many_arguments)]
fn worktree_row(
    rect: UiRect,
    worktree: &WorktreeInfo,
    repository: &RepositorySnapshot,
    main: bool,
    hovered: bool,
    enabled: bool,
    state: State<AppState>,
    store: State<GitStoreSnapshot>,
) -> Element {
    let current = worktree.path == repository.worktree_root;
    let mut buttons = Vec::new();
    if hovered && enabled && !worktree.bare {
        let open_state = state.clone();
        let open_store = store.clone();
        let path = worktree.path.clone();
        buttons.push(RowButton {
            key: format!("worktree-open-{}", path.display()),
            icon: "worktree-open",
            name: "Open Worktree (Add and Select)",
            on_click: Box::new(move || {
                git_actions::open_worktree(&open_state, &open_store, path.clone());
            }),
        });
        let add_state = state.clone();
        let path = worktree.path.clone();
        buttons.push(RowButton {
            key: format!("worktree-add-{}", path.display()),
            icon: "plus",
            name: "Add Worktree to Workspace",
            on_click: Box::new(move || git_actions::add_worktree_folder(&add_state, path.clone())),
        });
        if !main && !current && worktree.locked.is_none() {
            let source = repository.worktree_root.clone();
            let path = worktree.path.clone();
            let delete_state = state.clone();
            buttons.push(RowButton {
                key: format!("worktree-delete-{}", path.display()),
                icon: "trash",
                name: "Delete Worktree",
                on_click: Box::new(move || {
                    git_actions::request_remove_worktree(
                        &delete_state,
                        source.clone(),
                        path.clone(),
                        false,
                    )
                }),
            });
        }
    }
    let label_right = rect.right - BUTTON_W * buttons.len() as f32 - 12.0;
    let mut detail = worktree.branch.clone().unwrap_or_else(|| {
        if worktree.bare {
            "Bare repository".into()
        } else {
            format!(
                "Detached HEAD · {}",
                worktree.oid.chars().take(8).collect::<String>()
            )
        }
    });
    if current {
        detail.insert_str(0, "Current · ");
    }
    if let Some(reason) = &worktree.locked {
        detail.insert_str(0, "Locked · ");
        if !reason.is_empty() {
            detail.push_str(&format!(": {reason}"));
        }
    }
    let name = super::tabs::ellipsize(
        &repository_name(&worktree.path),
        (label_right - rect.left - 26.0).max(0.0),
        theme::UI_SIZE,
        400,
    );
    let detail_label = super::tabs::ellipsize(
        &detail,
        (label_right - rect.left - 26.0).max(0.0),
        theme::SMALL,
        400,
    );
    let open_state = state;
    let path = worktree.path.clone();
    let bare = worktree.bare;
    let mut row = panel(
        rect,
        if hovered {
            VisualStyle::filled(theme::c().active_line)
        } else {
            VisualStyle::default()
        },
    )
    .key(format!("git-worktree-{}", worktree.path.display()))
    .event_policy(EventPolicy::INTERACTIVE)
    .semantics(Semantics::new(SemanticRole::Button).name(format!(
        "{} · {detail}",
        crate::ui::display_path(&worktree.path)
    )))
    .cursor(if bare {
        CursorIcon::Default
    } else {
        CursorIcon::Pointer
    })
    .on_click(move || {
        if !bare {
            git_actions::open_worktree(&open_state, &store, path.clone());
        }
    })
    .child(text(
        UiRect::new(rect.left + 20.0, rect.top, label_right, rect.top + ROW_H),
        name,
        theme::mono(
            if current {
                theme::c().accent
            } else {
                theme::c().text
            },
            theme::UI_SIZE,
        ),
    ))
    .child(text(
        UiRect::new(rect.left + 20.0, rect.top + ROW_H, label_right, rect.bottom),
        detail_label,
        theme::mono(theme::c().text_dim, theme::SMALL),
    ));
    row = row_buttons(row, rect, rect.right - 8.0, buttons);
    row
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
        let icon_key = button.key.clone();
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
                lgui::core::UiId::owned(format!("{icon_key}-icon")),
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
    // Listed under its folder row: indented, by file name.
    nested: bool,
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
    let indent = if nested { 14.0 } else { 0.0 };
    let name_left = rect.left + 30.0 + indent;
    let name_viewport = UiRect::new(name_left, rect.top, rect.right - trailing_w, rect.bottom);
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
        git_actions::open_changes(
            &open_state,
            crate::git::DiffRequest {
                repository_root: open_repository.clone(),
                path: open_relative_path.clone(),
                target,
            },
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
        UiRect::new(
            rect.left + 12.0 + indent,
            rect.top,
            rect.left + 24.0 + indent,
            rect.bottom,
        ),
        indicator,
        theme::mono_bold(indicator_color, theme::SMALL),
    ))
    .child(path_label(
        name_viewport,
        &path,
        // Grouped rows sit under their folder row already.
        !nested,
        active || hovered,
    ));
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
    fn commit_card_messages_wrap_at_spaces_or_between_cjk_characters() {
        let width = super::super::tabs::measure("aaaa bbbb", theme::UI_SIZE, 400);
        assert_eq!(
            wrap_text("aaaa bbbb cccc\n\ndddd", width),
            ["aaaa bbbb", "cccc", "", "dddd"]
        );
        let cjk = "提交信息悬停卡片显示完整内容";
        let lines = wrap_text(cjk, width);
        assert!(lines.len() > 1);
        assert_eq!(lines.concat(), cjk);
        assert!(
            lines
                .iter()
                .all(|line| super::super::tabs::measure(line, theme::UI_SIZE, 400) <= width)
        );
        let long = "x".repeat(40);
        assert_eq!(wrap_text(&long, width).concat(), long);
    }

    #[test]
    fn grouping_lists_root_files_then_each_folder_with_its_files() {
        let files = ["README.md", "src/a.rs", "src/ui/b.rs", "src/c.rs"].map(PathBuf::from);
        let describe = |rows: Vec<ChangeRow>| {
            rows.into_iter()
                .map(|row| match row {
                    ChangeRow::Folder { path, files } => {
                        format!("[{}] {}", path.display(), files.len())
                    }
                    ChangeRow::File { path, nested } => {
                        format!("{}{}", if nested { "  " } else { "" }, path.display())
                    }
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(describe(change_rows(&files, false)).len(), 4);
        assert_eq!(
            describe(change_rows(&files, true)),
            [
                "README.md",
                "[src] 2",
                "  src/a.rs",
                "  src/c.rs",
                "[src/ui] 1",
                "  src/ui/b.rs"
            ]
        );
    }

    #[test]
    fn paths_lead_with_the_file_name() {
        assert_eq!(
            split_path(std::path::Path::new("src/api/client.ts")),
            ("client.ts".to_owned(), Some("src/api".to_owned()))
        );
        assert_eq!(
            split_path(std::path::Path::new("README.md")),
            ("README.md".to_owned(), None)
        );
    }

    #[test]
    fn ages_are_short_and_never_negative() {
        let now = 1_000_000_000;
        assert_eq!(relative_time(now, now + 30), "now");
        assert_eq!(relative_time(now, now - 90), "1m");
        assert_eq!(relative_time(now, now - 7_200), "2h");
        assert_eq!(relative_time(now, now - 3 * 86_400), "3d");
        assert_eq!(relative_time(now, now - 14 * 86_400), "2w");
        assert_eq!(relative_time(now, now - 400 * 86_400), "1y");
    }

    #[test]
    fn history_rows_show_state_and_the_expanded_commit_files() {
        let commit = |oid: &str| CommitSummary {
            oid: oid.into(),
            parents: Vec::new(),
            author: "a".into(),
            author_email: "a@b".into(),
            timestamp: 0,
            subject: oid.into(),
            decorations: Vec::new(),
        };
        assert_eq!(
            history_rows(None, None),
            [HistoryRow::Message("Loading commits…".into())]
        );
        let history = GitHistory {
            repository: PathBuf::from("repo"),
            head: Some("b".into()),
            commits: Ok(vec![commit("b"), commit("a")]),
        };
        let expanded = ExpandedCommit {
            oid: "b".into(),
            files: Some(Ok(vec![(ChangeKind::Modified, PathBuf::from("f"))])),
        };
        assert_eq!(
            history_rows(Some(&history), Some(&expanded)),
            [
                HistoryRow::Commit(commit("b"), true),
                HistoryRow::File {
                    oid: "b".into(),
                    kind: ChangeKind::Modified,
                    path: PathBuf::from("f"),
                },
                HistoryRow::Commit(commit("a"), false),
            ]
        );
    }

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
            worktrees: Default::default(),
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
