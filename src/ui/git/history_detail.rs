//! The selected commit's summary and changed files, beside the history list.

use std::path::{Path, PathBuf};

use lgui::core::{
    CursorIcon, EventPolicy, SemanticRole, Semantics, UiElement, UiFocusHandle, UiId, clip,
};
use lgui::prelude::{Element, UiRect, VisualStyle, panel, text};
use lgui::services::ServicesContextExt;

use crate::features::git::{ChangeKind, CommitSummary, StashInfo};
use crate::ui::git::commands::{Binding, Command};
use crate::ui::git::panel::{ROW_H, path_label, relative_time};
use crate::ui::{tabs, theme};

pub(crate) const WIDTH: f32 = 253.0;
const PAD: f32 = 16.0;

/// What the pane describes.
#[derive(Clone, Copy)]
pub(crate) enum Subject<'a> {
    Commit(&'a CommitSummary),
    Stash(&'a StashInfo),
}

type Files = Option<Result<Vec<(ChangeKind, PathBuf)>, String>>;

pub(crate) fn render(
    rect: UiRect,
    subject: Option<(Subject<'_>, &Files)>,
    repository: &Path,
    state: &Binding,
    editor_focus: &UiFocusHandle,
) -> Element {
    let root = panel(rect, VisualStyle::default()).key("git-history-detail");
    let left = rect.left + PAD;
    let right = (rect.right - PAD).max(left);
    let Some((subject, files)) = subject else {
        return clip(rect, 0.0, 0.0).child(root.child(text(
            UiRect::new(left, rect.top + 12.0, right, rect.top + 12.0 + ROW_H),
            "Select a commit or stash to see its changes.",
            theme::mono(theme::c().text_ghost, theme::SMALL),
        )));
    };

    let mut y = rect.top + 12.0;
    let (oid, hash, title, meta) = match subject {
        Subject::Commit(commit) => (
            commit.oid.clone(),
            commit.oid.chars().take(7).collect::<String>(),
            commit.subject.clone(),
            format!("{} · {}", commit.author, relative_time(commit.timestamp)),
        ),
        Subject::Stash(stash) => (
            stash.oid.clone(),
            format!("stash@{{{}}}", stash.index),
            stash.message.clone(),
            "Stash".to_owned(),
        ),
    };
    let copy_oid = oid.clone();
    let copy_rect = UiRect::new(right - 20.0, y, right, y + 20.0);
    let actions_w = if matches!(subject, Subject::Stash(_)) {
        66.0
    } else {
        0.0
    };
    let mut root = root
        .child(text(
            UiRect::new(left, y, copy_rect.left - actions_w, y + 20.0),
            hash,
            theme::mono(theme::c().accent, theme::SMALL + 1.0),
        ))
        .child(
            Element::new(move |cx| {
                let hovered = cx.context.interaction_flags(&cx.id).hovered;
                UiElement::panel(
                    cx.id,
                    copy_rect,
                    if hovered {
                        VisualStyle::filled(theme::c().surface).radius(3.0)
                    } else {
                        VisualStyle::default().radius(3.0)
                    },
                )
                .children(cx.children)
            })
            .key("git-history-detail-copy")
            .event_policy(EventPolicy::INTERACTIVE)
            .semantics(Semantics::new(SemanticRole::Button).name("Copy commit hash"))
            .cursor(CursorIcon::Pointer)
            .on_click_event(move |cx| {
                let _ = cx.application().clipboard().write_text(&copy_oid);
            })
            .child(crate::ui::drawer::drawer_icon(
                "git-history-detail-copy-icon",
                "copy",
                theme::icon_rect(copy_rect),
                theme::c().text_faint,
            )),
        );
    if let Subject::Stash(stash) = subject {
        let buttons = [
            ("git-stash-apply", "Apply Stash", false),
            ("git-stash-pop", "Pop Stash", true),
        ];
        for (index, (icon, name, pop)) in buttons.into_iter().enumerate() {
            let rect = UiRect::new(
                copy_rect.left - 66.0 + index as f32 * 22.0,
                y,
                copy_rect.left - 46.0 + index as f32 * 22.0,
                y + 20.0,
            );
            let (state, oid) = (state.clone(), stash.oid.clone());
            root = root.child(icon_button(rect, icon, name, move || {
                state.dispatch(Command::ApplyStash {
                    oid: oid.clone(),
                    pop,
                });
            }));
        }
        let rect = UiRect::new(copy_rect.left - 22.0, y, copy_rect.left - 2.0, y + 20.0);
        let (state, oid) = (state.clone(), stash.oid.clone());
        root = root.child(icon_button(rect, "trash", "Drop Stash", move || {
            state.dispatch(Command::RequestDropStash { oid: oid.clone() });
        }));
    }
    y += 26.0;
    root = root.child(text(
        UiRect::new(left, y, right, y + 20.0),
        tabs::ellipsize_sans(&title, right - left, 12.0, 400),
        theme::sans(theme::c().text, 12.0),
    ));
    y += 22.0;
    root = root.child(text(
        UiRect::new(left, y, right, y + 16.0),
        meta,
        theme::mono(theme::c().text_muted, theme::SMALL),
    ));
    y += 26.0;
    root = root.child(panel(
        UiRect::new(left, y, right, y + 1.0),
        VisualStyle::filled(theme::c().border),
    ));
    y += 4.0;

    let heading = match files {
        None => "Loading files…".to_owned(),
        Some(Err(error)) => error.clone(),
        Some(Ok(files)) => format!(
            "{} changed file{}",
            files.len(),
            if files.len() == 1 { "" } else { "s" }
        ),
    };
    root = root.child(text(
        UiRect::new(left, y, right, y + ROW_H),
        heading,
        theme::mono(theme::c().text_ghost, theme::SMALL),
    ));
    y += ROW_H;

    if let Some(Ok(files)) = files {
        let slots = ((rect.bottom - y) / ROW_H).floor().max(0.0) as usize;
        // Without room for every file, the last slot opens them all.
        let overflow = files.len() > slots;
        let shown = if overflow {
            slots.saturating_sub(1)
        } else {
            files.len()
        };
        let row_rect = |index: usize| {
            let top = y + index as f32 * ROW_H;
            UiRect::new(rect.left + 8.0, top, rect.right - 8.0, top + ROW_H)
        };
        for (index, (kind, path)) in files.iter().take(shown).enumerate() {
            root = root.child(file_row(
                row_rect(index),
                &oid,
                *kind,
                path,
                repository,
                state,
                editor_focus,
            ));
        }
        if overflow && slots > 0 {
            root = root.child(more_row(
                row_rect(shown),
                &oid,
                files.len() - shown,
                repository,
                state,
                editor_focus,
            ));
        }
    }
    clip(rect, 0.0, 0.0).child(root)
}

fn file_row(
    rect: UiRect,
    oid: &str,
    kind: ChangeKind,
    path: &Path,
    repository: &Path,
    state: &Binding,
    editor_focus: &UiFocusHandle,
) -> Element {
    let color = match kind {
        ChangeKind::Added | ChangeKind::Copied => theme::c().git.added,
        ChangeKind::Deleted => theme::c().git.deleted,
        _ => theme::c().git.modified,
    };
    let (state, focus) = (state.clone(), editor_focus.clone());
    let (root, oid_owned, open_path) =
        (repository.to_path_buf(), oid.to_owned(), path.to_path_buf());
    let key = format!("git-history-detail-{oid}-{}", path.display());
    let label = path.to_path_buf();
    Element::new(move |cx| {
        let hovered = cx.context.interaction_flags(&cx.id).hovered;
        UiElement::panel(
            cx.id,
            rect,
            if hovered {
                VisualStyle::filled(theme::c().surface).radius(3.0)
            } else {
                VisualStyle::default().radius(3.0)
            },
        )
        .children(cx.children)
    })
    .key(key)
    .event_policy(EventPolicy::INTERACTIVE)
    .semantics(Semantics::new(SemanticRole::Button).name(format!("Open {}", label.display())))
    .cursor(CursorIcon::Pointer)
    .on_click(move || {
        state.dispatch(Command::OpenCommitChanges {
            root: root.clone(),
            oid: oid_owned.clone(),
            focus: Some(open_path.clone()),
        });
        focus.focus();
    })
    .child(text(
        UiRect::new(rect.left + 6.0, rect.top, rect.left + 18.0, rect.bottom),
        kind.indicator(),
        theme::mono_bold(color, theme::SMALL),
    ))
    .child(path_label(
        UiRect::new(rect.left + 22.0, rect.top, rect.right - 6.0, rect.bottom),
        path,
        true,
        false,
    ))
}

fn more_row(
    rect: UiRect,
    oid: &str,
    hidden: usize,
    repository: &Path,
    state: &Binding,
    editor_focus: &UiFocusHandle,
) -> Element {
    let (state, focus) = (state.clone(), editor_focus.clone());
    let (root, oid_owned) = (repository.to_path_buf(), oid.to_owned());
    panel(rect, VisualStyle::default())
        .key(format!("git-history-detail-more-{oid}"))
        .event_policy(EventPolicy::INTERACTIVE)
        .semantics(Semantics::new(SemanticRole::Button).name("Open all changes"))
        .cursor(CursorIcon::Pointer)
        .on_click(move || {
            state.dispatch(Command::OpenCommitChanges {
                root: root.clone(),
                oid: oid_owned.clone(),
                focus: None,
            });
            focus.focus();
        })
        .child(text(
            UiRect::new(rect.left + 6.0, rect.top, rect.right - 6.0, rect.bottom),
            format!("+{hidden} more · View all changes"),
            theme::mono(theme::c().accent, theme::SMALL),
        ))
}

fn icon_button(
    rect: UiRect,
    icon: &'static str,
    name: &'static str,
    on_click: impl Fn() + Send + Sync + 'static,
) -> Element {
    Element::new(move |cx| {
        let hovered = cx.context.interaction_flags(&cx.id).hovered;
        UiElement::panel(
            cx.id,
            rect,
            if hovered {
                VisualStyle::filled(theme::c().surface).radius(3.0)
            } else {
                VisualStyle::default().radius(3.0)
            },
        )
        .children(cx.children)
    })
    .key(format!("git-history-detail-{icon}"))
    .event_policy(EventPolicy::INTERACTIVE)
    .semantics(Semantics::new(SemanticRole::Button).name(name))
    .cursor(CursorIcon::Pointer)
    .on_click(on_click)
    .child(crate::ui::drawer::drawer_icon(
        UiId::owned(format!("git-history-detail-{icon}-icon")),
        icon,
        theme::icon_rect(rect),
        theme::c().text_muted,
    ))
}
