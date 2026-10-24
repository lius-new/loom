//! Controls shared by the Git drawer's filter, file rows and commit composer.
use crate::features::git::GitStoreSnapshot;
use crate::ui::components::input::{self, AccessInputOptions, InputBinding, InputStyle};
use crate::ui::git::{
    commands::{Binding, Snapshot},
    panel as git_panel,
};
use crate::ui::theme;
use lgui::core::{
    CursorIcon, EventPolicy, Point, PointerButton, SemanticRole, Semantics, UiFocusHandle, UiId,
    UiPath, UiPathCommand, clip, clip_path,
};
use lgui::prelude::{Element, UiRect, VisualStyle, panel, text};
use std::sync::Arc;

pub const FILTER_ID: &str = "git-file-filter";
pub const HEADER_H: f32 = 80.0;

pub fn checkbox(
    rect: UiRect,
    key: String,
    label: String,
    checked: bool,
    enabled: bool,
    action: impl Fn() + Send + Sync + 'static,
) -> Element {
    let size = 12.0_f32
        .min(rect.height().max(0.0))
        .min(rect.width().max(0.0));
    let top = rect.top + (rect.height() - size) / 2.0;
    let square = UiRect::new(
        rect.left + (rect.width() - size) / 2.0,
        top,
        rect.left + (rect.width() + size) / 2.0,
        top + size,
    );
    let mut semantics = Semantics::new(SemanticRole::CheckBox).name(label);
    semantics.state.checked = Some(checked);
    semantics.state.disabled = !enabled;
    let mut root = panel(rect, VisualStyle::default())
        .key(key.clone())
        .event_policy(EventPolicy::INTERACTIVE)
        .semantics(semantics)
        .cursor(if enabled {
            CursorIcon::Pointer
        } else {
            CursorIcon::NotAllowed
        })
        .on_pointer_down_with_button(|cx, _, button| {
            if button == PointerButton::Left {
                cx.stop_propagation();
            }
        })
        .on_click(move |cx: &mut lgui::core::UiEventContext| {
            if enabled {
                action();
            }
            cx.stop_propagation();
        })
        .child(theme::bordered(
            square,
            if checked {
                theme::c().accent
            } else {
                theme::c().bg
            },
            if checked {
                theme::c().accent
            } else {
                theme::c().text_ghost
            },
            2.0,
            1.0,
        ));
    if checked {
        root = root.child(crate::ui::drawer::drawer_icon(
            UiId::owned(format!("{key}-check")),
            "check",
            square,
            theme::c().on_accent,
        ));
    }
    root
}

pub fn header(rect: UiRect, state: Binding, _store: Snapshot, focus: UiFocusHandle) -> Element {
    let app = state.get();
    let mut root = panel(rect, VisualStyle::default());
    let title = UiRect::new(
        rect.left + 16.0,
        rect.top,
        rect.right - 42.0,
        rect.top + 40.0,
    );
    root = root.child(text(
        title,
        "Changes",
        theme::sans_semibold(theme::c().text, 13.0),
    ));
    let grouped = app.ui.tree_view;
    let toggle = state.clone();
    root = root.child(
        panel(
            UiRect::new(
                rect.right - 38.0,
                rect.top + 7.0,
                rect.right - 10.0,
                rect.top + 35.0,
            ),
            VisualStyle::default().radius(4.0),
        )
        .key("git-group-by-folder")
        .event_policy(EventPolicy::INTERACTIVE)
        .cursor(CursorIcon::Pointer)
        .semantics(Semantics::new(SemanticRole::Button).name(if grouped {
            "View as List"
        } else {
            "Group by Folder"
        }))
        .on_click(move || toggle.update(|app| app.ui.tree_view = !app.ui.tree_view))
        .child(crate::ui::drawer::drawer_icon(
            "git-group-icon",
            if grouped { "list-flat" } else { "list-tree" },
            UiRect::new(
                rect.right - 30.0,
                rect.top + 13.0,
                rect.right - 16.0,
                rect.top + 27.0,
            ),
            theme::c().text_muted,
        )),
    );
    let filter = UiRect::new(
        rect.left + 12.0,
        rect.top + 42.0,
        (rect.right - 12.0).max(rect.left + 12.0),
        rect.top + 70.0,
    );
    root = root.child(
        theme::bordered(
            filter,
            theme::c().bg,
            if app.ui.file_filter.focused {
                theme::c().accent
            } else {
                theme::c().border
            },
            5.0,
            1.0,
        )
        .child(crate::ui::drawer::drawer_icon(
            "git-filter-search",
            "search",
            UiRect::new(
                filter.left + 8.0,
                filter.top + 7.0,
                filter.left + 22.0,
                filter.top + 21.0,
            ),
            theme::c().text_ghost,
        )),
    );
    let reset = state.clone();
    root = root.child(input::render_access(
        UiRect::new(
            filter.left + 24.0,
            filter.top,
            filter.right - 4.0,
            filter.bottom,
        ),
        UiId::new(FILTER_ID),
        focus,
        InputBinding::new_access(
            state.panel.clone(),
            |app| &app.ui.file_filter,
            |app| {
                app.ui.scroll = 0.0;
                &mut app.ui.file_filter
            },
        ),
        AccessInputOptions {
            label: "Filter changed files",
            placeholder: "Filter files…",
            style: InputStyle {
                text: theme::sans(theme::c().text, 12.0),
                placeholder: theme::sans(theme::c().text_ghost, 12.0),
                caret: theme::c().text,
                selection: theme::c().accent,
            },
            on_submit: None,
            on_cancel: Some(Arc::new(move || {
                reset.update(|app| {
                    app.ui.file_filter.clear();
                    app.ui.scroll = 0.0;
                })
            })),
            on_blur: None,
        },
    ));
    root
}

pub fn commit_reason(
    app: &super::state::GitPanelState,
    git: &GitStoreSnapshot,
) -> Option<&'static str> {
    if git.operation.is_some() {
        return Some("Git operation in progress");
    }
    let Some(repository) = git.active() else {
        return Some("No repository");
    };
    let sections = git
        .presentation
        .repositories
        .get(&repository.id)
        .map(|p| &p.sections);
    if sections.is_some_and(|s| !s[git_panel::CONFLICTS as usize].is_empty())
        || repository
            .files
            .values()
            .any(|file| file.conflict.is_some())
    {
        return Some("Resolve conflicts before committing");
    }
    let staged = sections.is_some_and(|s| !s[git_panel::STAGED as usize].is_empty())
        || repository
            .files
            .values()
            .any(|file| file.index != crate::features::git::ChangeKind::Unmodified);
    if !staged && !app.ui.commit_amend {
        return Some("No staged changes");
    }
    if app.ui.commit_input.text().trim().is_empty() && !app.ui.commit_amend {
        return Some("Enter a commit message");
    }
    None
}

pub fn composer(
    rect: UiRect,
    state: Binding,
    store: Snapshot,
    focus: UiFocusHandle,
    id: UiId,
) -> Element {
    let app = state.get();
    let git = store.get();
    let mut root = panel(rect, VisualStyle::filled(theme::c().sidebar))
        .child(crate::ui::components::divider::top(rect));
    let inner = UiRect::new(
        rect.left + 12.0,
        rect.top + 10.0,
        (rect.right - 12.0).max(rect.left + 12.0),
        rect.bottom - 10.0,
    );
    let label = UiRect::new(inner.left, inner.top, inner.right, inner.top + 22.0);
    root = root.child(text(
        UiRect::new(
            label.left,
            label.top,
            (label.right - 82.0).max(label.left),
            label.bottom,
        ),
        if rect.width() < 230.0 {
            "Message"
        } else {
            "Commit message"
        },
        theme::sans(theme::c().text_soft, 11.0),
    ));
    let amend = state.clone();
    root = root
        .child(checkbox(
            UiRect::new(
                label.right - 63.0,
                label.top,
                label.right - 43.0,
                label.bottom,
            ),
            "git-amend".into(),
            "Amend last commit".into(),
            app.ui.commit_amend,
            git.operation.is_none(),
            move || amend.update(|app| app.ui.commit_amend = !app.ui.commit_amend),
        ))
        .child(text(
            UiRect::new(label.right - 40.0, label.top, label.right, label.bottom),
            "Amend",
            theme::sans(theme::c().text_muted, 10.0),
        ));
    let button_bottom = inner.bottom;
    let button_top = (button_bottom - 32.0).max(label.bottom);
    let summary_top = (button_top - 28.0).max(label.bottom);
    let field = UiRect::new(
        inner.left,
        label.bottom + 5.0,
        inner.right,
        (summary_top - 5.0).max(label.bottom + 5.0),
    );
    root = root.child(
        theme::bordered(
            field,
            theme::c().bg,
            if app.ui.commit_input.focused {
                theme::c().accent
            } else {
                theme::c().border
            },
            6.0,
            1.0,
        )
        .child(git_panel::commit_input(field, state.clone(), focus, id)),
    );
    let staged = git
        .active()
        .and_then(|r| git.presentation.repositories.get(&r.id))
        .map_or(0, |p| p.sections[git_panel::STAGED as usize].len());
    root = root.child(text(
        UiRect::new(inner.left, summary_top, inner.right - 48.0, button_top),
        format!("{staged} staged"),
        theme::sans(theme::c().text_muted, 10.0),
    ));
    root = root
        .child(text(
            UiRect::new(
                inner.right - 36.0,
                summary_top,
                inner.right - 16.0,
                button_top,
            ),
            if cfg!(target_os = "macos") {
                "Cmd"
            } else {
                "Ctrl"
            },
            theme::sans(theme::c().text_ghost, 10.0),
        ))
        .child(crate::ui::drawer::drawer_icon(
            "git-commit-enter",
            "enter",
            UiRect::new(
                inner.right - 13.0,
                summary_top + 8.0,
                inner.right,
                summary_top + 21.0,
            ),
            theme::c().text_ghost,
        ));
    let options = UiRect::new(
        (inner.right - 28.0).max(inner.left),
        button_top,
        inner.right,
        button_bottom,
    );
    let enabled = commit_reason(&app, &git).is_none();
    let group = UiRect::new(inner.left, button_top, inner.right, button_bottom);
    root = root.child(
        clip_path(group, rounded_rect(group, 4.0))
            .child(git_panel::commit_button(
                UiRect::new(inner.left, button_top, options.left, button_bottom),
                state.clone(),
                store,
            ))
            .child(git_panel::commit_options_button(options, state, enabled))
            .child(panel(
                UiRect::new(
                    options.left,
                    button_top,
                    options.left + 0.5,
                    button_bottom,
                ),
                if enabled {
                    VisualStyle::filled(theme::c().on_accent).alpha(70)
                } else {
                    VisualStyle::filled(theme::c().border)
                },
            )),
    );
    clip(rect, 0.0, 0.0).child(root)
}

/// A rounded rectangle path, for clipping children to its outline.
fn rounded_rect(rect: UiRect, radius: f32) -> UiPath {
    let r = radius.min(rect.width() / 2.0).min(rect.height() / 2.0);
    let corner = |control: (f32, f32), to: (f32, f32)| UiPathCommand::QuadraticTo {
        control: Point::new(control.0, control.1),
        to: Point::new(to.0, to.1),
    };
    UiPath::new([
        UiPathCommand::MoveTo(Point::new(rect.left + r, rect.top)),
        UiPathCommand::LineTo(Point::new(rect.right - r, rect.top)),
        corner((rect.right, rect.top), (rect.right, rect.top + r)),
        UiPathCommand::LineTo(Point::new(rect.right, rect.bottom - r)),
        corner((rect.right, rect.bottom), (rect.right - r, rect.bottom)),
        UiPathCommand::LineTo(Point::new(rect.left + r, rect.bottom)),
        corner((rect.left, rect.bottom), (rect.left, rect.bottom - r)),
        UiPathCommand::LineTo(Point::new(rect.left, rect.top + r)),
        corner((rect.left, rect.top), (rect.left + r, rect.top)),
        UiPathCommand::Close,
    ])
}
