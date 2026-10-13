//! Source Control drawer.

use std::path::PathBuf;

use lgui::core::{
    CursorIcon, EventPolicy, PointerButton, SemanticRole, Semantics, UiElement, UiEventContext,
    UiEventKind, UiEventPayload, UiFocusHandle, UiId, WheelUnit, clip,
};
use lgui::prelude::{Element, UiRect, VisualStyle, panel, text};

use crate::features::git::model::{ExpandedCommit, GitHistory};
use crate::features::git::{
    ChangeKind, CommitSummary, ConflictSide, DiffTarget, FileState, RepositorySnapshot,
    RepositoryState, StashInfo, WorktreeInfo,
};
use crate::theme;
use crate::ui::components::input::{
    self, AccessInputOptions, InputBinding, InputState, InputStyle,
};
use crate::ui::components::scrollbar;
use crate::ui::git::commands::{Binding, Command, Snapshot};
use crate::ui::git::menu::{GitMenu, GitMenuTarget};
use crate::ui::git::state::GitPanelState;

pub(crate) const TOPLINE_H: f32 = theme::TABS_H;
pub(crate) const PROMPT_H: f32 = 32.0;
pub(crate) const SECTION_H: f32 = 22.0;
pub(crate) const ROW_H: f32 = 20.0;

/// The secondary sections stay anchored at the bottom. Expanded
/// details always have a 45% height budget, even without changes to review, with
/// enough room for the section headers even in a short drawer.
pub(crate) fn drawer_regions(
    body: UiRect,
    changes_height: f32,
    details_height: f32,
    detail_headers: usize,
) -> (UiRect, UiRect, UiRect) {
    let available = body.height().max(0.0);
    let content_available = (available - PROMPT_H).max(0.0);
    let headers_height = detail_headers as f32 * SECTION_H;
    let limit = (content_available * 0.45).max(headers_height);
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
pub(crate) enum ScrollRegion {
    Changes,
    Details,
}

impl ScrollRegion {
    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Changes => "git-changes-viewport",
            Self::Details => "git-details-viewport",
        }
    }

    pub(crate) fn scroll(self, app: &GitPanelState) -> f32 {
        match self {
            Self::Changes => app.ui.scroll,
            Self::Details => app.ui.details_scroll,
        }
    }

    pub(crate) fn set_scroll(self, app: &mut GitPanelState, value: f32) {
        match self {
            Self::Changes => app.ui.scroll = value,
            Self::Details => app.ui.details_scroll = value,
        }
        app.ui.commit_card = None;
    }

    pub(crate) fn drag(self, app: &GitPanelState) -> (bool, f32) {
        match self {
            Self::Changes => (app.ui.scrollbar_dragging, app.ui.scrollbar_drag_offset),
            Self::Details => (
                app.ui.details_scrollbar_dragging,
                app.ui.details_scrollbar_drag_offset,
            ),
        }
    }

    pub(crate) fn set_drag(self, app: &mut GitPanelState, dragging: bool, offset: f32) {
        match self {
            Self::Changes => {
                app.ui.scrollbar_dragging = dragging;
                app.ui.scrollbar_drag_offset = offset;
            }
            Self::Details => {
                app.ui.details_scrollbar_dragging = dragging;
                app.ui.details_scrollbar_drag_offset = offset;
            }
        }
    }
}

pub(crate) fn scroll_region(
    rect: UiRect,
    content: Vec<Element>,
    content_bottom: f32,
    scroll: f32,
    max_scroll: f32,
    state: Binding,
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
    if max_scroll > 0.0 && (app.ui.hovered || region.drag(&app).0) {
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
    state: Binding,
    store: Snapshot,
    editor_focus: UiFocusHandle,
    commit_focus: UiFocusHandle,
    commit_id: UiId,
) -> Element {
    let app = state.get();
    let frame = state.frame();
    let resizing = frame.resizing;
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
    let grouped = app.ui.tree_view;
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
                group_state.update(|app| app.ui.tree_view = !app.ui.tree_view);
            }),
        }],
    ));
    // With several repositories, name the active one and offer the others.
    if git.repositories.len() > 1
        && let Some(repository) = git.active()
    {
        let name = repository_name(&repository.worktree_root);
        let label_left =
            rect.left + 12.0 + crate::ui::tabs::measure("CHANGES", theme::SMALL, 700) + 14.0;
        let picker = UiRect::new(
            label_left - 4.0,
            topline.top + 4.0,
            (label_left + crate::ui::tabs::measure(&name, theme::SMALL, 400) + 26.0)
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
                    app.ui.menu = Some(GitMenu {
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
            .child(crate::ui::drawer::drawer_icon(
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
        (title, filter, files, change_rows(files, app.ui.tree_view))
    });
    let history = app
        .model
        .history()
        .filter(|history| history.repository == repository.worktree_root);
    let history_rows = history_rows(history, app.model.history_commit());
    let section_height = |filter: u8, rows: usize| {
        SECTION_H
            + if app.ui.collapsed_sections.contains(&filter) {
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
    let scroll = app.ui.scroll.clamp(0.0, max_scroll);
    let mut y = content_rect.top + 8.0;
    // Rows the pointer can hover, in content coordinates (before scrolling).
    let mut hover_targets = Vec::new();
    for (title, filter, files, rows) in &change_sections {
        let (title, filter) = (*title, *filter);
        if files.is_empty() {
            continue;
        }
        let collapsed = app.ui.collapsed_sections.contains(&filter);
        let section_rect = UiRect::new(rect.left, y, content_rect.right, y + SECTION_H);
        hover_targets.push((section_rect, GitHover::Section(filter)));
        let buttons = if app.ui.hover == Some(GitHover::Section(filter)) {
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
                    let hovered = app.ui.hover.as_ref() == Some(&hover);
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
                    let hovered = app.ui.hover.as_ref() == Some(&hover);
                    hover_targets.push((row_rect, hover));
                    let target = if filter == STAGED {
                        DiffTarget::HeadToIndex
                    } else {
                        DiffTarget::IndexToWorktree
                    };
                    let active = [frame.active_diff.as_ref(), frame.active_diff_focus.as_ref()]
                        .into_iter()
                        .flatten()
                        .any(|focus| {
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
    let scroll = app.ui.details_scroll.clamp(0.0, details_max_scroll);
    let mut y = content_rect.top;
    if repository.worktrees.len() > 1 {
        let collapsed = app.ui.collapsed_sections.contains(&WORKTREES);
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
                let hovered = app.ui.hover.as_ref() == Some(&hover);
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
        let collapsed = app.ui.collapsed_sections.contains(&STASHES);
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
                let hovered = app.ui.hover.as_ref() == Some(&hover);
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
        let collapsed = app.ui.collapsed_sections.contains(&HISTORY);
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
                let hovered = hover.is_some() && app.ui.hover == hover;
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
                let changed = app.ui.hover != next;
                if changed {
                    entered = true;
                    app.ui.commit_card = None;
                }
                app.ui.hover = next;
                changed
            });
            if entered && let Some((anchor, GitHover::Commit(oid))) = target {
                {
                    hover_state.dispatch(Command::HoverCommit {
                        root: hover_root.clone(),
                        oid: oid.clone(),
                        anchor,
                    });
                };
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
        if app.ui.commit_input.focused {
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

pub(crate) fn visible_rows(
    rows_top: f32,
    viewport_top: f32,
    viewport_bottom: f32,
    count: usize,
) -> std::ops::Range<usize> {
    visible_rows_with_height(rows_top, viewport_top, viewport_bottom, count, ROW_H)
}

pub(crate) fn visible_worktree_rows(
    rows_top: f32,
    viewport_top: f32,
    viewport_bottom: f32,
    count: usize,
) -> std::ops::Range<usize> {
    visible_rows_with_height(rows_top, viewport_top, viewport_bottom, count, ROW_H * 2.0)
}

pub(crate) fn visible_rows_with_height(
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

pub(crate) fn git_vertical_scrollbar(
    rect: UiRect,
    content_bottom: f32,
    scroll: f32,
    state: Binding,
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

pub(crate) fn git_resize_handle(rect: UiRect, state: Binding, resizing: bool) -> Element {
    if !state.frame().resizable {
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
            drag_start.dispatch(Command::ResizeStart);
        }
    })
    .on_pointer_drag(move |_cx, pointer| {
        drag_move.dispatch(Command::Resize {
            width: (pointer.point.x - drawer_left)
                .clamp(theme::SIDEBAR_MIN_W, theme::SIDEBAR_MAX_W),
        });
    })
    .on_pointer_up(move |_cx, _pointer| {
        drag_end.dispatch(Command::ResizeEnd);
    })
}

pub(crate) fn commit_input(
    rect: UiRect,
    state: Binding,
    focus: UiFocusHandle,
    id: UiId,
) -> Element {
    let placeholder = if state.get().ui.commit_amend {
        "Amend message (empty keeps it)"
    } else {
        "Commit message"
    };
    input::render_access(
        rect,
        id,
        focus,
        InputBinding::new_access(
            state.panel.clone(),
            commit_input_state,
            commit_input_state_mut,
        ),
        AccessInputOptions {
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

pub(crate) fn commit_input_state(app: &GitPanelState) -> &InputState {
    &app.ui.commit_input
}

pub(crate) fn commit_input_state_mut(app: &mut GitPanelState) -> &mut InputState {
    &mut app.ui.commit_input
}

/// Opens the commit options menu: amend, sign-off and undo.
pub(crate) fn commit_options_button(rect: UiRect, state: Binding) -> Element {
    let app = state.get();
    let open = app
        .ui
        .menu
        .as_ref()
        .is_some_and(|menu| menu.target == GitMenuTarget::Commit);
    // Amend and sign-off stay visible while the menu is closed.
    let color = if app.ui.commit_amend || app.ui.commit_signoff {
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
            app.ui.menu = Some(GitMenu {
                pos: (rect.left, rect.top),
                target: GitMenuTarget::Commit,
                hover: None,
            });
        });
    })
    .child(crate::ui::drawer::drawer_icon(
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

pub(crate) fn commit_button(rect: UiRect, state: Binding, _store: Snapshot) -> Element {
    let app = state.get();
    let has_message = app.ui.commit_amend || !app.ui.commit_input.text().trim().is_empty();
    let click_state = state.clone();
    panel(rect, VisualStyle::default())
        .key("git-commit-button")
        .event_policy(EventPolicy::INTERACTIVE)
        .semantics(
            Semantics::new(SemanticRole::Button).name(if app.ui.commit_amend {
                "Amend last commit"
            } else {
                "Commit changes"
            }),
        )
        .cursor(CursorIcon::Pointer)
        .on_click(move || {
            click_state.dispatch(Command::Commit);
        })
        .child(crate::ui::drawer::drawer_icon(
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

/// A history commit's hover card, beside its row.
#[derive(Clone, Debug, PartialEq)]
pub struct CommitCard {
    pub oid: String,
    /// The commit's row on screen when the card was requested.
    pub anchor: UiRect,
    pub details: Result<crate::features::git::CommitDetails, String>,
}

pub(crate) const CARD_W: f32 = 420.0;
pub(crate) const CARD_PAD: f32 = 12.0;
pub(crate) const CARD_LINE_H: f32 = 18.0;
pub(crate) const CARD_MAX_MESSAGE_LINES: usize = 16;

/// The hovered history commit's card, to the right of Source Control.
pub fn render_commit_card(viewport: UiRect, app: &GitPanelState) -> Option<Element> {
    let card = app.ui.commit_card.as_ref()?;
    if app.ui.hover.as_ref() != Some(&GitHover::Commit(card.oid.clone())) {
        return None;
    }
    let summary = app
        .model
        .history()
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
        crate::features::git::diff_document::short_oid(&card.oid).to_owned(),
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
                    let piece_w = crate::ui::tabs::measure(
                        &piece,
                        theme::UI_SIZE,
                        if bold { 700 } else { 400 },
                    );
                    let piece = crate::ui::tabs::ellipsize(
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
pub(crate) fn wrap_text(text: &str, width: f32) -> Vec<String> {
    let fits = |line: &str| crate::ui::tabs::measure(line, theme::UI_SIZE, 400) <= width;
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

pub(crate) const BANNER_H: f32 = 30.0;

/// The folder name shown for a repository.
pub(crate) fn repository_name(root: &std::path::Path) -> String {
    root.file_name().map_or_else(
        || crate::ui::display_path(root),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// A row of a change section.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ChangeRow<'a> {
    /// A folder and the section's files directly in it.
    Folder { path: PathBuf, files: Vec<PathBuf> },
    /// A file; `nested` when listed under its folder row.
    File { path: &'a PathBuf, nested: bool },
}

/// `files` as rows: as given, or grouped under one row per folder (files
/// at the repository root first, without a folder row).
pub(crate) fn change_rows(files: &[PathBuf], grouped: bool) -> Vec<ChangeRow<'_>> {
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
pub(crate) fn group_buttons(
    filter: u8,
    paths: Vec<PathBuf>,
    key: &str,
    state: &Binding,
    store: &Snapshot,
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
pub(crate) fn folder_row(
    rect: UiRect,
    filter: u8,
    path: &std::path::Path,
    files: Vec<PathBuf>,
    hovered: bool,
    state: &Binding,
    store: &Snapshot,
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
    .child(crate::ui::drawer::drawer_icon(
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
pub(crate) fn path_label(
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
    let name_w = crate::ui::tabs::measure(&name, theme::SMALL, 400) + TEXT_PAD;
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
        let folder_w = crate::ui::tabs::measure(&folder, theme::SMALL, 400) + TEXT_PAD;
        label = label.child(text(
            UiRect::new(left, viewport.top, left + folder_w, viewport.bottom),
            folder,
            folder_style,
        ));
    }
    label
}

/// Extra width past a measured text so its last glyph is never clipped.
pub(crate) const TEXT_PAD: f32 = 6.0;

/// `path` as its file name and its folder, `None` at the repository root.
pub(crate) fn split_path(path: &std::path::Path) -> (String, Option<String>) {
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
pub(crate) enum HistoryRow {
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

pub(crate) fn history_rows(
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
pub(crate) fn relative_time(now: i64, timestamp: i64) -> String {
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

pub(crate) fn history_row(
    rect: UiRect,
    row: &HistoryRow,
    hovered: bool,
    now: i64,
    repository: &std::path::Path,
    state: &Binding,
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
            let age_w = crate::ui::tabs::measure(&age, 9.0, 400) + 6.0;
            panel(rect, background)
                .key(format!("git-commit-{}", commit.oid))
                .event_policy(EventPolicy::INTERACTIVE)
                .semantics(Semantics::new(SemanticRole::Button).name(commit.subject.clone()))
                .cursor(CursorIcon::Pointer)
                .on_click(move || {
                    // A right-click that opened this row's menu is not a click.
                    let mut menu_open = false;
                    toggle_state.try_update(|app| {
                        menu_open = app.ui.menu.is_some();
                        false
                    });
                    if !menu_open {
                        {
                            toggle_state.dispatch(Command::ToggleCommit {
                                root: root.clone(),
                                oid: oid.clone(),
                            });
                        };
                    }
                })
                .on_pointer_down_with_button(move |_cx, pointer, button| {
                    if button == PointerButton::Right {
                        let target = GitMenuTarget::HistoryCommit {
                            repository: menu_root.clone(),
                            oid: menu_oid.clone(),
                        };
                        menu_state.update(move |app| {
                            app.ui.menu = Some(GitMenu {
                                pos: (pointer.point.x, pointer.point.y),
                                target,
                                hover: None,
                            });
                        });
                    }
                })
                .child(crate::ui::drawer::drawer_icon(
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
                    {
                        open_state.dispatch(Command::OpenCommitChanges {
                            root: root.clone(),
                            oid: open_oid.clone(),
                            focus: Some(open_path.clone()),
                        });
                    };
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
pub(crate) fn section_header(
    rect: UiRect,
    filter: u8,
    title: &'static str,
    count: usize,
    collapsed: bool,
    buttons: Vec<RowButton>,
    state: Binding,
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
                if !app.ui.collapsed_sections.insert(filter) {
                    app.ui.collapsed_sections.remove(&filter);
                }
                if filter >= STASHES {
                    app.ui.details_scroll = 0.0;
                    app.ui.commit_card = None;
                }
            });
        })
        .child(crate::ui::drawer::drawer_icon(
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
pub(crate) fn worktree_row(
    rect: UiRect,
    worktree: &WorktreeInfo,
    repository: &RepositorySnapshot,
    main: bool,
    hovered: bool,
    enabled: bool,
    state: Binding,
    _store: Snapshot,
) -> Element {
    let current = worktree.path == repository.worktree_root;
    let mut buttons = Vec::new();
    if hovered && enabled && !worktree.bare {
        let open_state = state.clone();
        let path = worktree.path.clone();
        buttons.push(RowButton {
            key: format!("worktree-open-{}", path.display()),
            icon: "worktree-open",
            name: "Open Worktree (Add and Select)",
            on_click: Box::new(move || {
                open_state.dispatch(Command::OpenWorktree { path: path.clone() });
            }),
        });
        let add_state = state.clone();
        let path = worktree.path.clone();
        buttons.push(RowButton {
            key: format!("worktree-add-{}", path.display()),
            icon: "plus",
            name: "Add Worktree to Workspace",
            on_click: Box::new(move || {
                add_state.dispatch(Command::AddWorktreeFolder { path: path.clone() });
            }),
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
                    delete_state.dispatch(Command::RequestRemoveWorktree {
                        repository: source.clone(),
                        path: path.clone(),
                        force: false,
                    });
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
    let name = crate::ui::tabs::ellipsize(
        &repository_name(&worktree.path),
        (label_right - rect.left - 26.0).max(0.0),
        theme::UI_SIZE,
        400,
    );
    let detail_label = crate::ui::tabs::ellipsize(
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
            open_state.dispatch(Command::OpenWorktree { path: path.clone() });
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
pub(crate) fn stash_row(
    rect: UiRect,
    stash: &StashInfo,
    hovered: bool,
    state: Binding,
    _store: Snapshot,
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
    .child(crate::ui::drawer::drawer_icon(
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

        let oid = stash.oid.clone();
        RowButton {
            key: format!("git-stash-{}-{action:?}", stash.oid),
            icon,
            name,
            on_click: Box::new(move || match action {
                StashAction::Apply => {
                    action_state.dispatch(Command::ApplyStash {
                        oid: oid.clone(),
                        pop: false,
                    });
                }
                StashAction::Pop => {
                    action_state.dispatch(Command::ApplyStash {
                        oid: oid.clone(),
                        pop: true,
                    });
                }
                StashAction::Drop => {
                    action_state.dispatch(Command::RequestDropStash { oid: oid.clone() });
                }
            }),
        }
    })
    .collect();
    row = row_buttons(row, rect, rect.right - 8.0, buttons);
    row
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum StashAction {
    Apply,
    Pop,
    Drop,
}

/// The banner label for an operation in progress, with rebase progress.
pub(crate) fn operation_label(repository: &RepositorySnapshot) -> Option<String> {
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
pub(crate) fn operation_banner(
    rect: UiRect,
    label: String,
    operation: RepositoryState,
    conflicts: bool,
    idle: bool,
    state: Binding,
    _store: Snapshot,
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
    let (abort, abort_left) = text_button(right, rect, abort_label, false, idle, move || {
        abort_state.dispatch(Command::RequestAbort);
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
                state.dispatch(Command::ContinueOperation);
            },
        );
        banner = banner.child(button);
        right = left - 6.0;
    }
    let label_w = crate::ui::tabs::measure(&label, theme::SMALL, 700) + 6.0;
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
pub(crate) fn text_button(
    right: f32,
    bar: UiRect,
    label: &'static str,
    primary: bool,
    enabled: bool,
    on_click: impl Fn() + Send + Sync + 'static,
) -> (Element, f32) {
    let width = crate::ui::tabs::measure(label, theme::SMALL, 600) + 16.0;
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

pub(crate) const BUTTON_W: f32 = 20.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RowAction {
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
pub(crate) fn section_actions(section: u8) -> &'static [RowAction] {
    match section {
        STAGED => &[RowAction::Unstage],
        UNSTAGED | UNTRACKED => &[RowAction::Discard, RowAction::Stage],
        // Marking every conflict resolved at once is too easy to regret.
        _ => &[],
    }
}

/// Buttons on a file row, left to right.
pub(crate) fn file_actions(section: u8) -> &'static [RowAction] {
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
    pub(crate) fn icon(self) -> &'static str {
        match self {
            Self::OpenFile => "go-to-file",
            Self::Stage => "plus",
            Self::Unstage => "minus",
            Self::Discard => "discard",
            Self::KeepCurrent => "arrow-left",
            Self::TakeIncoming => "arrow-right",
        }
    }

    pub(crate) fn section_name(self, section: u8) -> &'static str {
        match self {
            Self::OpenFile | Self::KeepCurrent | Self::TakeIncoming => self.file_name(section),
            Self::Stage => "Stage All Changes",
            Self::Unstage => "Unstage All Changes",
            Self::Discard if section == UNTRACKED => "Delete All Untracked Files",
            Self::Discard => "Discard All Changes",
        }
    }

    pub(crate) fn file_name(self, section: u8) -> &'static str {
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
    pub(crate) fn run(self, state: &Binding, _store: &Snapshot, section: u8, paths: Vec<PathBuf>) {
        match self {
            Self::OpenFile => {}
            Self::Stage => {
                state.dispatch(Command::StagePaths { paths });
            }
            Self::Unstage => {
                state.dispatch(Command::UnstagePaths { paths });
            }
            Self::Discard if section == UNTRACKED => {
                state.dispatch(Command::RequestDiscard {
                    tracked: Vec::new(),
                    untracked: paths,
                });
            }
            Self::Discard => {
                state.dispatch(Command::RequestDiscard {
                    tracked: paths,
                    untracked: Vec::new(),
                });
            }
            Self::KeepCurrent | Self::TakeIncoming => {
                let side = if self == Self::KeepCurrent {
                    ConflictSide::Ours
                } else {
                    ConflictSide::Theirs
                };
                for path in paths {
                    state.dispatch(Command::ResolveConflict { path, side });
                }
            }
        }
    }
}

pub(crate) struct RowButton {
    key: String,
    icon: &'static str,
    name: &'static str,
    on_click: Box<dyn Fn() + Send + Sync>,
}

/// Add `buttons` to `row`, right-aligned to end at `right`.
pub(crate) fn row_buttons(
    mut row: Element,
    rect: UiRect,
    right: f32,
    buttons: Vec<RowButton>,
) -> Element {
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
            .on_pointer_down_with_button(move |cx, _pointer, button| {
                if button == PointerButton::Left {
                    cx.stop_propagation();
                }
            })
            .on_click(move |cx: &mut UiEventContext| {
                on_click();
                cx.stop_propagation();
            })
            .child(crate::ui::drawer::drawer_icon(
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
pub(crate) fn file_row(
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
    state: Binding,
    store: Snapshot,
    editor_focus: UiFocusHandle,
) -> Element {
    let staged = section == STAGED;
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
    .on_pointer_down_with_button(move |cx, pointer, button| {
        match button {
            PointerButton::Left => {
                // Like Explorer, defer opening until release so a drag can
                // land on a pane or create a split instead of opening Changes.
                let request = crate::features::git::DiffRequest {
                    repository_root: menu_repository.clone(),
                    path: menu_path.clone(),
                    target,
                };
                let point = (pointer.point.x, pointer.point.y);
                menu_state.dispatch(Command::PressChange {
                    request,
                    untracked: section == UNTRACKED,
                    point,
                });
            }
            PointerButton::Right => {
                let target = GitMenuTarget::File {
                    repository: menu_repository.clone(),
                    path: menu_path.clone(),
                    section,
                };
                menu_state.update(move |app| {
                    app.ui.menu = Some(GitMenu {
                        pos: (pointer.point.x, pointer.point.y),
                        target,
                        hover: None,
                    });
                });
            }
            _ => return,
        }
        cx.stop_propagation();
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
                        if action_state.dispatch(Command::OpenFile {
                            path: open_path.clone(),
                        }) {
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
