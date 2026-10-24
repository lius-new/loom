//! The selected commit's summary and changed files, beside the history list.

use std::path::{Path, PathBuf};

use lgui::core::{
    CursorIcon, EventPolicy, SemanticRole, Semantics, UiElement, UiFocusHandle, clip,
};
use lgui::prelude::{Element, UiRect, VisualStyle, panel, text};
use lgui::services::ServicesContextExt;

use crate::features::git::{ChangeKind, CommitSummary};
use crate::ui::git::commands::{Binding, Command};
use crate::ui::git::panel::{ROW_H, path_label, relative_time};
use crate::ui::{tabs, theme};

pub(crate) const WIDTH: f32 = 253.0;
const PAD: f32 = 16.0;

type Files = Option<Result<Vec<(ChangeKind, PathBuf)>, String>>;

pub(crate) fn render(
    rect: UiRect,
    commit: Option<(&CommitSummary, &Files)>,
    repository: &Path,
    state: &Binding,
    editor_focus: &UiFocusHandle,
) -> Element {
    let root = panel(rect, VisualStyle::default())
        .key("git-history-detail")
        .child(panel(
            UiRect::new(rect.left, rect.top, rect.left + 1.0, rect.bottom),
            VisualStyle::filled(theme::c().border),
        ));
    let left = rect.left + PAD;
    let right = (rect.right - PAD).max(left);
    let Some((commit, files)) = commit else {
        return clip(rect, 0.0, 0.0).child(root.child(text(
            UiRect::new(left, rect.top + 12.0, right, rect.top + 12.0 + ROW_H),
            "Select a commit to see its changes.",
            theme::mono(theme::c().text_ghost, theme::SMALL),
        )));
    };

    let mut y = rect.top + 12.0;
    let hash: String = commit.oid.chars().take(7).collect();
    let copy_oid = commit.oid.clone();
    let copy_rect = UiRect::new(right - 20.0, y, right, y + 20.0);
    let mut root = root
        .child(text(
            UiRect::new(left, y, copy_rect.left, y + 20.0),
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
    y += 26.0;
    root = root.child(text(
        UiRect::new(left, y, right, y + 20.0),
        tabs::ellipsize_sans(&commit.subject, right - left, 12.0, 400),
        theme::sans(theme::c().text, 12.0),
    ));
    y += 22.0;
    root = root.child(text(
        UiRect::new(left, y, right, y + 16.0),
        format!("{} · {}", commit.author, relative_time(commit.timestamp)),
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
                &commit.oid,
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
                &commit.oid,
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
