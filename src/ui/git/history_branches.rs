//! The reference navigator beside the history: branches, remotes, tags and
//! stashes. Choosing one decides what the history and its detail pane show.

use std::collections::{BTreeMap, HashSet};

use lgui::core::{
    CursorIcon, EventPolicy, PointerButton, SemanticRole, Semantics, UiElement, UiId, WheelUnit,
    clip,
};
use lgui::prelude::{Color, Element, UiRect, VisualStyle, panel, text};

use crate::features::git::{BranchInfo, RepositorySnapshot, StashInfo, TagInfo};
use crate::ui::git::commands::{Binding, Command, Snapshot};
use crate::ui::git::menu::{GitMenu, GitMenuTarget};
use crate::ui::{tabs, theme};

pub(crate) const WIDTH: f32 = 164.0;
const ROW_H: f32 = 22.0;
const HEADING_H: f32 = 24.0;
const PAD: f32 = 8.0;
const INDENT: f32 = 12.0;

/// The prefix that marks a tag in `GitUiState::history_scope`.
pub(crate) const TAG_PREFIX: &str = "tag:";

/// Branches grouped by the `/` in their names, so `feature/a` and `feature/b`
/// share a `feature` folder and a remote's branches sit under its name.
#[derive(Default)]
struct Node<'a> {
    folders: BTreeMap<String, Node<'a>>,
    leaves: Vec<(String, &'a BranchInfo)>,
    count: usize,
}

impl<'a> Node<'a> {
    fn insert(&mut self, segments: &[&str], branch: &'a BranchInfo) {
        self.count += 1;
        match segments {
            [] => {}
            [name] => self.leaves.push(((*name).to_owned(), branch)),
            [folder, rest @ ..] => self
                .folders
                .entry((*folder).to_owned())
                .or_default()
                .insert(rest, branch),
        }
    }

    /// Whether `name` is one of the branches in this folder or below it.
    fn contains(&self, name: &str) -> bool {
        self.leaves.iter().any(|(_, info)| info.name == name)
            || self.folders.values().any(|child| child.contains(name))
    }

    fn flatten(
        &self,
        kind: &str,
        path: &str,
        depth: usize,
        collapsed: &HashSet<String>,
        selected: Option<&str>,
        out: &mut Vec<Entry<'a>>,
    ) {
        for (name, child) in &self.folders {
            let path = if path.is_empty() {
                name.clone()
            } else {
                format!("{path}/{name}")
            };
            let key = format!("{kind}{path}");
            let expanded = !collapsed.contains(&key);
            out.push(Entry::Folder {
                key,
                name: name.clone(),
                depth,
                expanded,
                count: child.count,
                holds_selection: !expanded && selected.is_some_and(|name| child.contains(name)),
            });
            if expanded {
                child.flatten(kind, &path, depth + 1, collapsed, selected, out);
            }
        }
        let mut leaves = self.leaves.clone();
        leaves.sort_by(|a, b| a.0.cmp(&b.0));
        out.extend(leaves.into_iter().map(|(display, info)| Entry::Branch {
            info,
            depth,
            display,
        }));
    }
}

/// A top-level `Local` or `Remote` folder, holding its branches as a tree.
fn group<'a>(
    key: &str,
    name: &str,
    kind: &str,
    branches: impl Iterator<Item = &'a BranchInfo>,
    collapsed: &HashSet<String>,
    selected: Option<&str>,
    out: &mut Vec<Entry<'a>>,
) {
    let mut root = Node::default();
    for branch in branches {
        let segments = branch.name.split('/').collect::<Vec<_>>();
        root.insert(&segments, branch);
    }
    if root.count == 0 {
        return;
    }
    let expanded = !collapsed.contains(key);
    out.push(Entry::Folder {
        key: key.to_owned(),
        name: name.to_owned(),
        depth: 0,
        expanded,
        count: root.count,
        holds_selection: !expanded && selected.is_some_and(|name| root.contains(name)),
    });
    if expanded {
        root.flatten(kind, "", 1, collapsed, selected, out);
    }
}

/// What a heading's `+` button creates.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Create {
    Branch,
    Tag,
    Stash,
}

enum Entry<'a> {
    Heading(&'static str, Option<Create>),
    Folder {
        key: String,
        name: String,
        depth: usize,
        expanded: bool,
        count: usize,
        holds_selection: bool,
    },
    Branch {
        info: &'a BranchInfo,
        depth: usize,
        display: String,
    },
    Tag(&'a TagInfo),
    Stash(&'a StashInfo),
}

impl Entry<'_> {
    fn height(&self) -> f32 {
        match self {
            Self::Heading(..) => HEADING_H,
            _ => ROW_H,
        }
    }
}

fn entries<'a>(
    branches: &'a [BranchInfo],
    repository: &'a RepositorySnapshot,
    collapsed: &HashSet<String>,
    selected: Option<&str>,
) -> Vec<Entry<'a>> {
    let mut entries = vec![Entry::Heading("BRANCHES", Some(Create::Branch))];
    group(
        LOCAL,
        "Local",
        "b:",
        branches.iter().filter(|branch| !branch.remote),
        collapsed,
        selected,
        &mut entries,
    );
    // A remote's own HEAD alias repeats its default branch.
    group(
        REMOTE,
        "Remote",
        "r:",
        branches
            .iter()
            .filter(|branch| branch.remote && !branch.name.ends_with("/HEAD")),
        collapsed,
        selected,
        &mut entries,
    );
    // Always listed, so a repository without any can still create the first.
    entries.push(Entry::Heading("TAGS", Some(Create::Tag)));
    entries.extend(repository.tags.iter().map(Entry::Tag));
    entries.push(Entry::Heading("STASHES", Some(Create::Stash)));
    entries.extend(repository.stashes.iter().map(Entry::Stash));
    entries
}

/// Folder keys of the two branch groups; remotes start folded.
pub(crate) const LOCAL: &str = "g:local";
pub(crate) const REMOTE: &str = "g:remote";

pub(crate) fn content_height(
    branches: &[BranchInfo],
    repository: &RepositorySnapshot,
    collapsed: &HashSet<String>,
) -> f32 {
    entries(branches, repository, collapsed, None)
        .iter()
        .map(Entry::height)
        .sum::<f32>()
        + PAD
}

pub(crate) fn render(
    rect: UiRect,
    repository: &RepositorySnapshot,
    branches: &[BranchInfo],
    scope: Option<&str>,
    stash: Option<&str>,
    tag_target: Option<&str>,
    collapsed: &HashSet<String>,
    scroll: f32,
    state: &Binding,
    store: &Snapshot,
) -> Element {
    let max_scroll = (content_height(branches, repository, collapsed) - rect.height()).max(0.0);
    let scroll = scroll.clamp(0.0, max_scroll);
    let current = branches
        .iter()
        .find(|branch| branch.current)
        .map(|branch| branch.name.as_str());
    // Choosing a stash leaves the history on its branch, which stops being the
    // selection; the stash is.
    let selected = scope.or(current).filter(|_| stash.is_none());
    let mut layer = clip(rect, 0.0, 0.0);
    let mut y = rect.top + PAD - scroll;
    for entry in entries(branches, repository, collapsed, selected) {
        let height = entry.height();
        let row = UiRect::new(rect.left, y, rect.right, y + height);
        y += height;
        if row.bottom < rect.top || row.top > rect.bottom {
            continue;
        }
        layer = layer.child(match entry {
            Entry::Heading(title, create) => {
                heading_row(row, title, create, repository, tag_target, state, store)
            }
            Entry::Folder {
                key,
                name,
                depth,
                expanded,
                count,
                holds_selection,
            } => folder_row(
                row,
                key,
                name,
                depth,
                expanded,
                count,
                holds_selection,
                state,
            ),
            Entry::Branch {
                info,
                depth,
                display,
            } => branch_row(
                row,
                repository,
                info,
                &display,
                depth,
                selected == Some(info.name.as_str()),
                state,
            ),
            Entry::Tag(tag) => tag_row(
                row,
                repository,
                tag,
                scope.and_then(|scope| scope.strip_prefix(TAG_PREFIX)) == Some(tag.name.as_str())
                    && stash.is_none(),
                state,
            ),
            Entry::Stash(info) => stash_row(
                row,
                repository,
                info,
                stash == Some(info.oid.as_str()),
                state,
            ),
        });
    }
    let wheel_state = state.clone();
    panel(rect, VisualStyle::default())
        .key("git-history-branches")
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

/// A selectable row: its background follows the selection and the pointer.
fn row(
    key: String,
    rect: UiRect,
    selected: bool,
    name: String,
    on_click: impl Fn() + Send + Sync + 'static,
) -> Element {
    let mut semantics = Semantics::new(SemanticRole::Button).name(name);
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
        UiElement::panel(cx.id, rect, fill).children(cx.children)
    })
    .key(key)
    .event_policy(EventPolicy::INTERACTIVE)
    .semantics(semantics)
    .cursor(CursorIcon::Pointer)
    .on_click(on_click)
}

fn color(selected: bool) -> Color {
    if selected {
        theme::c().accent
    } else {
        theme::c().text_muted
    }
}

/// A row's icon and label, the label leaving `trailing` pixels free.
fn label(
    rect: UiRect,
    icon: &'static str,
    key: &str,
    value: &str,
    selected: bool,
    trailing: f32,
) -> (Element, Element) {
    let color = color(selected);
    (
        crate::ui::drawer::drawer_icon(
            UiId::owned(format!("git-history-ref-icon-{key}")),
            icon,
            theme::icon_rect(UiRect::new(
                rect.left + 4.0,
                rect.top + 3.0,
                rect.left + 18.0,
                rect.bottom - 3.0,
            )),
            color,
        ),
        text(
            UiRect::new(rect.left + 22.0, rect.top, rect.right, rect.bottom),
            tabs::ellipsize(value, rect.width() - 26.0 - trailing, theme::SMALL, 400),
            theme::mono(color, theme::SMALL),
        )
        .into(),
    )
}

fn inner(rect: UiRect) -> UiRect {
    UiRect::new(rect.left + 6.0, rect.top, rect.right - 7.0, rect.bottom)
}

fn branch_row(
    rect: UiRect,
    repository: &RepositorySnapshot,
    branch: &BranchInfo,
    display: &str,
    depth: usize,
    selected: bool,
    state: &Binding,
) -> Element {
    let inner = inner(rect);
    let root = repository.worktree_root.clone();
    // The current branch is HEAD, which needs no scope of its own.
    let name = (!branch.current).then(|| branch.name.clone());
    let click_state = state.clone();
    // Another checkout of this branch: open it, add it, or delete it.
    let worktree = branch
        .worktree_path
        .clone()
        .filter(|path| *path != repository.worktree_root);
    let indented = UiRect::new(
        inner.left + depth as f32 * INDENT,
        inner.top,
        inner.right,
        inner.bottom,
    );
    let (icon, text_element) = label(
        indented,
        if branch.remote { "cloud" } else { "git-branch" },
        &branch.name,
        display,
        selected,
        if worktree.is_some() { 18.0 } else { 0.0 },
    );
    let mut element = row(
        format!("git-history-branch-{}", branch.name),
        inner,
        selected,
        format!("Show commits of {}", branch.name),
        move || {
            click_state.dispatch(Command::SelectHistoryBranch {
                root: root.clone(),
                name: name.clone(),
            });
        },
    )
    .child(icon)
    .child(text_element);
    if let Some(path) = worktree {
        let removable = repository
            .worktrees
            .iter()
            .position(|worktree| worktree.path == path)
            .is_some_and(|index| index > 0 && repository.worktrees[index].locked.is_none());
        let target = GitMenuTarget::Worktree {
            repository: repository.worktree_root.clone(),
            path,
            removable,
        };
        let open = {
            let state = state.clone();
            move |x: f32, y: f32| {
                let target = target.clone();
                state.update(move |app| {
                    app.ui.clear_hover_cards();
                    app.ui.menu = Some(GitMenu {
                        pos: (x, y),
                        target,
                        hover: None,
                    });
                });
            }
        };
        let icon_rect = UiRect::new(
            inner.right - 20.0,
            inner.top + 2.0,
            inner.right - 2.0,
            inner.bottom - 2.0,
        );
        let button_open = open.clone();
        element = element
            .on_pointer_down_with_button(move |cx, pointer, button| {
                if button == PointerButton::Right {
                    open(pointer.point.x, pointer.point.y);
                    cx.stop_propagation();
                }
            })
            .child(
                panel(icon_rect, VisualStyle::default().radius(3.0))
                    .key(format!("git-history-worktree-{}", branch.name))
                    .event_policy(EventPolicy::INTERACTIVE)
                    .semantics(
                        Semantics::new(SemanticRole::Button)
                            .name(format!("Worktree of {}", branch.name)),
                    )
                    .cursor(CursorIcon::Pointer)
                    .on_click(move || button_open(icon_rect.left, icon_rect.bottom))
                    .child(crate::ui::drawer::drawer_icon(
                        UiId::owned(format!("git-history-worktree-icon-{}", branch.name)),
                        "folder",
                        theme::icon_rect(icon_rect),
                        theme::c().text_faint,
                    )),
            );
    }
    element
}

fn tag_row(
    rect: UiRect,
    repository: &RepositorySnapshot,
    tag: &TagInfo,
    selected: bool,
    state: &Binding,
) -> Element {
    let inner = inner(rect);
    let root = repository.worktree_root.clone();
    let name = format!("{TAG_PREFIX}{}", tag.name);
    let click_state = state.clone();
    let click_root = root.clone();
    let (icon, text_element) = label(inner, "git-ref-tag", &tag.name, &tag.name, selected, 0.0);
    let element = row(
        format!("git-history-tag-{}", tag.name),
        inner,
        selected,
        format!("Show commits of tag {}", tag.name),
        move || {
            click_state.dispatch(Command::SelectHistoryBranch {
                root: click_root.clone(),
                name: Some(name.clone()),
            });
        },
    )
    .child(icon)
    .child(text_element);
    crate::ui::git::panel::tag_context_menu(element, &root, tag.name.clone(), state)
}

fn stash_row(
    rect: UiRect,
    repository: &RepositorySnapshot,
    stash: &StashInfo,
    selected: bool,
    state: &Binding,
) -> Element {
    let inner = inner(rect);
    let (oid, root) = (stash.oid.clone(), repository.worktree_root.clone());
    let click_state = state.clone();
    let (icon, text_element) = label(
        inner,
        "git-stash",
        &stash.oid,
        &stash.message,
        selected,
        0.0,
    );
    row(
        format!("git-history-stash-{}", stash.oid),
        inner,
        selected,
        format!("Show stash {}", stash.message),
        move || {
            click_state.dispatch(Command::SelectHistoryStash {
                root: root.clone(),
                oid: oid.clone(),
            });
        },
    )
    .child(icon)
    .child(text_element)
}

/// A branch folder: click to fold or unfold what is inside it.
fn folder_row(
    rect: UiRect,
    key: String,
    name: String,
    depth: usize,
    expanded: bool,
    count: usize,
    holds_selection: bool,
    state: &Binding,
) -> Element {
    let inner = inner(rect);
    let left = inner.left + depth as f32 * INDENT;
    let toggle = state.clone();
    let id = key.clone();
    let label = format!("{name}  {count}");
    row(
        format!("git-history-folder-{key}"),
        inner,
        false,
        format!("{name} folder"),
        move || {
            toggle.update(|app| {
                if !app.ui.history_collapsed.remove(&id) {
                    app.ui.history_collapsed.insert(id.clone());
                }
            });
        },
    )
    .child(crate::ui::drawer::drawer_icon(
        UiId::owned(format!("git-history-folder-icon-{key}")),
        if expanded {
            "chevron-down"
        } else {
            "chevron-right"
        },
        theme::icon_rect(UiRect::new(
            left + 2.0,
            inner.top + 5.0,
            left + 14.0,
            inner.bottom - 5.0,
        )),
        theme::c().text_faint,
    ))
    .child(text(
        UiRect::new(left + 18.0, inner.top, inner.right, inner.bottom),
        tabs::ellipsize(&label, inner.right - left - 22.0, theme::SMALL, 400),
        theme::mono(theme::c().text_muted, theme::SMALL),
    ))
    // The selected branch is folded away; keep a trace of where it is.
    .child(text(
        UiRect::new(
            inner.right - 16.0,
            inner.top,
            inner.right - 4.0,
            inner.bottom,
        ),
        if holds_selection { "\u{25cf}" } else { "" },
        theme::mono(theme::c().accent, theme::SMALL),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn branch(name: &str, remote: bool) -> BranchInfo {
        BranchInfo {
            name: name.into(),
            remote,
            current: false,
            oid: String::new(),
            upstream: None,
            worktree_path: None,
        }
    }

    fn shape(entries: &[Entry<'_>]) -> Vec<String> {
        entries
            .iter()
            .map(|entry| match entry {
                Entry::Folder {
                    name,
                    depth,
                    expanded,
                    count,
                    ..
                } => format!(
                    "{}{name}/ {count}{}",
                    " ".repeat(*depth),
                    if *expanded { "" } else { " (folded)" }
                ),
                Entry::Branch { display, depth, .. } => format!("{}{display}", " ".repeat(*depth)),
                _ => String::new(),
            })
            .collect()
    }

    #[test]
    fn slashes_in_branch_names_become_folders_with_folders_first() {
        let branches = [
            branch("main", false),
            branch("feature/b", false),
            branch("feature/a", false),
            branch("fix/deep/x", false),
        ];
        let mut out = Vec::new();
        group(
            LOCAL,
            "Local",
            "b:",
            branches.iter(),
            &HashSet::new(),
            None,
            &mut out,
        );
        assert_eq!(
            shape(&out),
            [
                "Local/ 4",
                " feature/ 2",
                "  a",
                "  b",
                " fix/ 1",
                "  deep/ 1",
                "   x",
                " main"
            ]
        );
    }

    #[test]
    fn remote_branches_group_under_their_remote_and_folders_can_fold() {
        let branches = [
            branch("origin/main", true),
            branch("origin/feature/a", true),
        ];
        let collapsed = HashSet::from(["r:origin/feature".to_owned()]);
        let mut out = Vec::new();
        group(
            REMOTE,
            "Remote",
            "r:",
            branches.iter(),
            &collapsed,
            None,
            &mut out,
        );
        assert_eq!(
            shape(&out),
            ["Remote/ 2", " origin/ 2", "  feature/ 1 (folded)", "  main"]
        );
    }

    #[test]
    fn a_folded_group_marks_that_it_holds_the_selected_branch() {
        let branches = [branch("origin/main", true), branch("origin/dev", true)];
        let collapsed = HashSet::from([REMOTE.to_owned()]);
        for (selected, marked) in [
            (Some("origin/dev"), true),
            (Some("other"), false),
            (None, false),
        ] {
            let mut out = Vec::new();
            group(
                REMOTE,
                "Remote",
                "r:",
                branches.iter(),
                &collapsed,
                selected,
                &mut out,
            );
            assert_eq!(out.len(), 1);
            assert!(matches!(
                out[0],
                Entry::Folder { holds_selection, .. } if holds_selection == marked
            ));
        }
    }
}

/// A group title, with the button that creates a new member of the group.
fn heading_row(
    rect: UiRect,
    title: &'static str,
    create: Option<Create>,
    repository: &RepositorySnapshot,
    tag_target: Option<&str>,
    state: &Binding,
    store: &Snapshot,
) -> Element {
    let mut heading = panel(rect, VisualStyle::default()).child(text(
        UiRect::new(rect.left + 14.0, rect.top, rect.right - 30.0, rect.bottom),
        title,
        theme::mono(theme::c().text_ghost, theme::SMALL),
    ));
    let Some(create) = create else {
        return heading.into();
    };
    let idle = store.get().operation.is_none();
    let (name, enabled) = match create {
        Create::Branch => ("New Branch or Worktree", idle),
        Create::Tag => ("New Tag at Selected Commit", idle && tag_target.is_some()),
        Create::Stash => ("Stash Changes", idle && !repository.files.is_empty()),
    };
    let button = UiRect::new(
        rect.right - 26.0,
        rect.top + 2.0,
        rect.right - 6.0,
        rect.bottom - 2.0,
    );
    let (state, store) = (state.clone(), store.clone());
    let root = repository.worktree_root.clone();
    let target = tag_target.map(str::to_owned);
    let mut semantics = Semantics::new(SemanticRole::Button).name(name);
    semantics.state.disabled = !enabled;
    heading = heading.child(
        Element::new(move |cx| {
            let hovered = enabled && cx.context.interaction_flags(&cx.id).hovered;
            UiElement::panel(
                cx.id,
                button,
                if hovered {
                    VisualStyle::filled(theme::c().surface).radius(3.0)
                } else {
                    VisualStyle::default().radius(3.0)
                },
            )
            .children(cx.children)
        })
        .key(format!("git-history-create-{title}"))
        .event_policy(EventPolicy::INTERACTIVE)
        .semantics(semantics)
        .cursor(if enabled {
            CursorIcon::Pointer
        } else {
            CursorIcon::Default
        })
        .on_click(move || {
            if !enabled {
                return;
            }
            match create {
                Create::Branch => {
                    crate::ui::git::branch_picker::open(&state, &store, rect.left);
                }
                Create::Tag => {
                    if let Some(target) = &target {
                        state.dispatch(Command::RequestCreateTag {
                            root: root.clone(),
                            branch: None,
                            target: target.clone(),
                        });
                    }
                }
                Create::Stash => {
                    state.dispatch(Command::StashAll);
                }
            }
        })
        .child(crate::ui::drawer::drawer_icon(
            UiId::owned(format!("git-history-create-icon-{title}")),
            "plus",
            theme::icon_rect(button),
            if enabled {
                theme::c().text_muted
            } else {
                theme::c().text_ghost
            },
        )),
    );
    heading.into()
}
