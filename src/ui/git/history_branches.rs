//! The branch list beside the history: choose whose commits it shows.

use std::path::Path;

use lgui::core::{
    CursorIcon, EventPolicy, SemanticRole, Semantics, UiElement, UiId, WheelUnit, clip,
};
use lgui::prelude::{Element, UiRect, VisualStyle, panel, text};

use crate::features::git::BranchInfo;
use crate::ui::git::commands::{Binding, Command};
use crate::ui::{tabs, theme};

pub(crate) const WIDTH: f32 = 164.0;
const ROW_H: f32 = 22.0;
const HEADING_H: f32 = 24.0;
const PAD: f32 = 8.0;

enum Entry<'a> {
    Heading(&'static str),
    Branch(&'a BranchInfo),
}

impl Entry<'_> {
    fn height(&self) -> f32 {
        match self {
            Self::Heading(_) => HEADING_H,
            Self::Branch(_) => ROW_H,
        }
    }
}

fn entries(branches: &[BranchInfo]) -> Vec<Entry<'_>> {
    let mut entries = vec![Entry::Heading("BRANCHES")];
    entries.extend(
        branches
            .iter()
            .filter(|branch| !branch.remote)
            .map(Entry::Branch),
    );
    // A remote's own HEAD alias repeats its default branch.
    let remotes = branches
        .iter()
        .filter(|branch| branch.remote && !branch.name.ends_with("/HEAD"))
        .collect::<Vec<_>>();
    if !remotes.is_empty() {
        entries.push(Entry::Heading("REMOTES"));
        entries.extend(remotes.into_iter().map(Entry::Branch));
    }
    entries
}

pub(crate) fn content_height(branches: &[BranchInfo]) -> f32 {
    entries(branches).iter().map(Entry::height).sum::<f32>() + PAD
}

pub(crate) fn render(
    rect: UiRect,
    repository: &Path,
    branches: &[BranchInfo],
    scope: Option<&str>,
    scroll: f32,
    state: &Binding,
) -> Element {
    let max_scroll = (content_height(branches) - rect.height()).max(0.0);
    let scroll = scroll.clamp(0.0, max_scroll);
    let current = branches
        .iter()
        .find(|branch| branch.current)
        .map(|branch| branch.name.as_str());
    let selected = scope.or(current);
    let mut layer = clip(rect, 0.0, 0.0);
    let mut y = rect.top + PAD - scroll;
    for entry in entries(branches) {
        let height = entry.height();
        let row = UiRect::new(rect.left, y, rect.right, y + height);
        y += height;
        if row.bottom < rect.top || row.top > rect.bottom {
            continue;
        }
        layer = layer.child(match entry {
            Entry::Heading(title) => text(
                UiRect::new(row.left + 14.0, row.top, row.right - 8.0, row.bottom),
                title,
                theme::mono(theme::c().text_ghost, theme::SMALL),
            )
            .into(),
            Entry::Branch(branch) => branch_row(
                row,
                repository,
                branch,
                selected == Some(branch.name.as_str()),
                state,
            ),
        });
    }
    let wheel_state = state.clone();
    panel(rect, VisualStyle::default())
        .key("git-history-branches")
        .child(panel(
            UiRect::new(rect.right - 1.0, rect.top, rect.right, rect.bottom),
            VisualStyle::filled(theme::c().border),
        ))
        .child(layer)
        .on_wheel(move |cx, delta| {
            let step = match delta.unit {
                WheelUnit::Lines => delta.y * ROW_H * 3.0,
                WheelUnit::Pixels => delta.y,
            };
            wheel_state.update(|app| {
                app.ui.history_branch_scroll = (scroll - step).clamp(0.0, max_scroll);
            });
            cx.stop_propagation();
        })
}

fn branch_row(
    rect: UiRect,
    repository: &Path,
    branch: &BranchInfo,
    selected: bool,
    state: &Binding,
) -> Element {
    let inner = UiRect::new(rect.left + 6.0, rect.top, rect.right - 7.0, rect.bottom);
    let (state, root) = (state.clone(), repository.to_path_buf());
    // The current branch is HEAD, which needs no scope of its own.
    let name = (!branch.current).then(|| branch.name.clone());
    let label = tabs::ellipsize(&branch.name, inner.width() - 24.0, theme::SMALL, 400);
    let color = if selected {
        theme::c().accent
    } else {
        theme::c().text_muted
    };
    let mut semantics =
        Semantics::new(SemanticRole::Button).name(format!("Show commits of {}", branch.name));
    semantics.state.selected = selected;
    Element::new(move |cx| {
        let hovered = cx.context.interaction_flags(&cx.id).hovered;
        let fill = if selected {
            VisualStyle::filled(theme::c().active_line).radius(3.0)
        } else if hovered {
            VisualStyle::filled(theme::c().surface).radius(3.0)
        } else {
            VisualStyle::default().radius(3.0)
        };
        UiElement::panel(cx.id, inner, fill).children(cx.children)
    })
    .key(format!("git-history-branch-{}", branch.name))
    .event_policy(EventPolicy::INTERACTIVE)
    .semantics(semantics)
    .cursor(CursorIcon::Pointer)
    .on_click(move || {
        state.dispatch(Command::SelectHistoryBranch {
            root: root.clone(),
            name: name.clone(),
        });
    })
    .child(crate::ui::drawer::drawer_icon(
        UiId::owned(format!("git-history-branch-icon-{}", branch.name)),
        if branch.remote { "cloud" } else { "git-branch" },
        theme::icon_rect(UiRect::new(
            inner.left + 4.0,
            inner.top + 3.0,
            inner.left + 18.0,
            inner.bottom - 3.0,
        )),
        color,
    ))
    .child(text(
        UiRect::new(inner.left + 22.0, inner.top, inner.right, inner.bottom),
        label,
        theme::mono(color, theme::SMALL),
    ))
}
