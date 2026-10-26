//! The selected commit's summary and changed files, beside the history list.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use lgui::core::{
    CursorIcon, EventPolicy, SemanticRole, Semantics, UiElement, UiFocusHandle, UiId, clip,
};
use lgui::prelude::{Element, UiRect, VisualStyle, panel, text};
use lgui::services::ServicesContextExt;

use lgui::prelude::Color;

use crate::features::git::{ChangeKind, CommitDetails, CommitSummary, StashInfo};
use crate::ui::git::commands::{Binding, Command};
use crate::ui::git::history_branches::pin_slots;
use crate::ui::git::panel::{
    ROW_H, ScrollRegion, change_color, change_path_label, relative_time, scroll_region,
    wrap_text_sized,
};
use crate::ui::{tabs, theme};

pub(crate) const WIDTH: f32 = 253.0;
const PAD: f32 = 16.0;
const INDENT: f32 = 12.0;
type Details<'a> = Option<&'a Result<CommitDetails, String>>;

const LINE_H: f32 = 15.0;
/// Room that always stays for the file tree under the header.
const MIN_FILES_H: f32 = 110.0;
/// The hash row and the gap under it.
const TOP_ROW_H: f32 = 26.0;

/// One line under the hash row: coloured pieces drawn one after another.
enum Line {
    Text(Vec<(String, Color)>),
    Gap,
}

impl Line {
    fn height(&self) -> f32 {
        match self {
            Self::Text(_) => LINE_H,
            Self::Gap => 6.0,
        }
    }
}

struct Layout {
    lines: Vec<Line>,
    /// Where the file tree starts, counted from the pane's top.
    files_top: f32,
}

fn plural(count: usize, word: &str) -> String {
    format!("{count} {word}{}", if count == 1 { "" } else { "s" })
}

/// Everything above the file tree, sized to the pane so that the tree always
/// keeps room and the message takes what is left.
fn layout(rect: UiRect, subject: Subject<'_>, details: Details<'_>) -> Layout {
    let inner = (rect.width() - PAD * 2.0).max(0.0);
    let c = theme::c();
    let fit = |value: String| tabs::ellipsize(&value, inner, theme::SMALL, 400);
    let mut meta: Vec<Line> = Vec::new();
    match details {
        None => meta.push(Line::Text(vec![("Loading details…".into(), c.text_ghost)])),
        Some(Err(error)) => meta.push(Line::Text(vec![(fit(error.clone()), c.error_text)])),
        Some(Ok(details)) => {
            if let Subject::Commit(commit) = subject {
                meta.push(Line::Text(vec![(
                    fit(format!("{} <{}>", details.author, details.author_email)),
                    c.text_bright,
                )]));
                meta.push(Line::Text(vec![(
                    fit(format!(
                        "{} · {}",
                        details.date,
                        relative_time(commit.timestamp)
                    )),
                    c.text_muted,
                )]));
            } else {
                meta.push(Line::Text(vec![(fit(details.date.clone()), c.text_muted)]));
            }
            if details.files_changed > 0 {
                let mut stats = vec![(plural(details.files_changed, "file"), c.text_soft)];
                if details.insertions > 0 {
                    stats.push((format!("  +{}", details.insertions), c.git.added_text));
                }
                if details.deletions > 0 {
                    stats.push((format!("  −{}", details.deletions), c.git.deleted));
                }
                meta.push(Line::Text(stats));
            }
        }
    }
    if let Subject::Commit(commit) = subject {
        let refs = commit
            .decorations
            .iter()
            .map(|value| crate::ui::git::history_badges::display_name(value))
            .filter(|name| !name.is_empty())
            .collect::<Vec<_>>();
        if !refs.is_empty() {
            meta.push(Line::Text(vec![(fit(refs.join(", ")), c.accent)]));
        }
        if !commit.parents.is_empty() {
            let parents = commit
                .parents
                .iter()
                .map(|oid| oid.chars().take(7).collect::<String>())
                .collect::<Vec<_>>()
                .join(" ");
            meta.push(Line::Text(vec![(
                fit(format!("parent {parents}")),
                c.text_dim,
            )]));
        }
    }

    let message = match (subject, details) {
        (Subject::Commit(_), Some(Ok(details))) => details.message.clone(),
        (Subject::Commit(commit), _) => commit.subject.clone(),
        (Subject::Stash(stash), _) => stash.message.clone(),
    };
    // What the message may take: the pane less the fixed parts and the tree.
    let fixed =
        12.0 + TOP_ROW_H + 6.0 + meta.iter().map(Line::height).sum::<f32>() + 6.0 + 5.0 + ROW_H;
    let budget = ((rect.height() - fixed - MIN_FILES_H) / LINE_H).floor();
    let max_lines = (budget as isize).clamp(2, 14) as usize;
    let mut wrapped = wrap_text_sized(&message.replace('\t', "    "), inner, theme::SMALL);
    if wrapped.len() > max_lines {
        wrapped.truncate(max_lines);
        if let Some(last) = wrapped.last_mut() {
            last.push('…');
        }
    }
    // The subject paragraph stands out from the body under it.
    let mut subject_done = false;
    let mut lines = Vec::new();
    for line in wrapped {
        if line.is_empty() {
            subject_done = true;
        }
        let color = if subject_done { c.text_soft } else { c.text };
        lines.push(Line::Text(vec![(line, color)]));
    }
    lines.push(Line::Gap);
    lines.extend(meta);

    let files_top =
        12.0 + TOP_ROW_H + lines.iter().map(Line::height).sum::<f32>() + 6.0 + 5.0 + ROW_H;
    Layout { lines, files_top }
}

/// The scrolling file tree inside the detail pane `rect`.
pub(crate) fn files_rect(rect: UiRect, subject: Subject<'_>, details: Details<'_>) -> UiRect {
    UiRect::new(
        rect.left,
        (rect.top + layout(rect, subject, details).files_top).min(rect.bottom),
        rect.right,
        rect.bottom,
    )
}

/// What the pane describes.
#[derive(Clone, Copy)]
pub(crate) enum Subject<'a> {
    Commit(&'a CommitSummary),
    Stash(&'a StashInfo),
}

impl Subject<'_> {
    pub(crate) fn oid(&self) -> &str {
        match self {
            Self::Commit(commit) => &commit.oid,
            Self::Stash(stash) => &stash.oid,
        }
    }
}

type Files = Option<Result<Vec<(ChangeKind, PathBuf)>, String>>;

pub(crate) fn render(
    rect: UiRect,
    subject: Option<(Subject<'_>, &Files)>,
    details: Details<'_>,
    repository: &Path,
    state: &Binding,
    editor_focus: &UiFocusHandle,
    scroll: f32,
    collapsed: &HashSet<String>,
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
    let (oid, hash) = match subject {
        Subject::Commit(commit) => (
            commit.oid.clone(),
            commit.oid.chars().take(7).collect::<String>(),
        ),
        Subject::Stash(stash) => (stash.oid.clone(), format!("stash@{{{}}}", stash.index)),
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
    let layout = layout(rect, subject, details);
    y += TOP_ROW_H;
    for line in &layout.lines {
        if let Line::Text(parts) = line {
            let mut x = left;
            for (piece, color) in parts {
                let width = tabs::measure(piece, theme::SMALL, 400) + 1.0;
                root = root.child(text(
                    UiRect::new(x, y, (x + width).min(right), y + LINE_H),
                    piece.clone(),
                    theme::mono(*color, theme::SMALL),
                ));
                x += width;
            }
        }
        y += line.height();
    }
    y += 6.0;
    root = root.child(panel(
        UiRect::new(left, y, right, y + 1.0),
        VisualStyle::filled(theme::c().border),
    ));
    y += 5.0;

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
        let body = UiRect::new(rect.left, y, rect.right, rect.bottom);
        debug_assert!((body.top - files_rect(rect, subject, details).top).abs() < 0.01);
        root = root.child(file_tree(
            body,
            files,
            &oid,
            repository,
            state,
            editor_focus,
            scroll,
            collapsed,
        ));
    }
    clip(rect, 0.0, 0.0).child(root)
}

/// The changed files as a folder tree.
#[derive(Default)]
struct Dir {
    dirs: BTreeMap<String, Dir>,
    files: Vec<(String, ChangeKind, PathBuf)>,
    count: usize,
}

impl Dir {
    fn insert(&mut self, folders: &[String], name: String, kind: ChangeKind, path: PathBuf) {
        self.count += 1;
        match folders {
            [] => self.files.push((name, kind, path)),
            [first, rest @ ..] => self
                .dirs
                .entry(first.clone())
                .or_default()
                .insert(rest, name, kind, path),
        }
    }

    /// Folders that hold only one folder join into one row, as in `src/ui/git`.
    fn flatten(
        &self,
        prefix: &str,
        depth: usize,
        collapsed: &HashSet<String>,
        out: &mut Vec<Item>,
    ) {
        for (name, child) in &self.dirs {
            let (mut label, mut path, mut node) = (
                name.clone(),
                if prefix.is_empty() {
                    name.clone()
                } else {
                    format!("{prefix}/{name}")
                },
                child,
            );
            while node.files.is_empty() && node.dirs.len() == 1 {
                let (next, inner) = node.dirs.iter().next().expect("one folder");
                label = format!("{label}/{next}");
                path = format!("{path}/{next}");
                node = inner;
            }
            let key = format!("f:{path}");
            let expanded = !collapsed.contains(&key);
            out.push(Item::Folder {
                key,
                name: label,
                depth,
                expanded,
                count: node.count,
            });
            if expanded {
                node.flatten(&path, depth + 1, collapsed, out);
            }
        }
        let mut files = self.files.clone();
        files.sort_by(|a, b| a.0.cmp(&b.0));
        out.extend(files.into_iter().map(|(name, kind, path)| Item::File {
            name,
            kind,
            path,
            depth,
        }));
    }
}

#[derive(Clone)]
enum Item {
    Folder {
        key: String,
        name: String,
        depth: usize,
        expanded: bool,
        count: usize,
    },
    File {
        name: String,
        kind: ChangeKind,
        path: PathBuf,
        depth: usize,
    },
}

impl Item {
    fn depth(&self) -> usize {
        match self {
            Self::Folder { depth, .. } | Self::File { depth, .. } => *depth,
        }
    }
}

fn items(files: &[(ChangeKind, PathBuf)], collapsed: &HashSet<String>) -> Vec<Item> {
    let mut root = Dir::default();
    for (kind, path) in files {
        let folders = path
            .parent()
            .map(|parent| {
                parent
                    .components()
                    .map(|part| part.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let name = path.file_name().map_or_else(
            || path.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        );
        root.insert(&folders, name, *kind, path.clone());
    }
    let mut out = Vec::new();
    root.flatten("", 0, collapsed, &mut out);
    out
}

/// The file tree: scrolls under the header, with folders pinned at the top.
fn file_tree(
    body: UiRect,
    files: &[(ChangeKind, PathBuf)],
    oid: &str,
    repository: &Path,
    state: &Binding,
    editor_focus: &UiFocusHandle,
    scroll: f32,
    collapsed: &HashSet<String>,
) -> Element {
    let rows = items(files, collapsed);
    let mut tops = Vec::with_capacity(rows.len());
    let mut y = body.top;
    for _ in &rows {
        tops.push(y);
        y += ROW_H;
    }
    let content_bottom = y + 8.0;
    let max_scroll = (content_bottom - body.bottom).max(0.0);
    let scroll = scroll.clamp(0.0, max_scroll);
    let (view_top, view_bottom) = (body.top + scroll, body.bottom + scroll);
    let kinds = rows
        .iter()
        .map(|row| (matches!(row, Item::Folder { .. }), row.depth()))
        .collect::<Vec<_>>();
    let pinned = pin_slots(&kinds, &tops, y, view_top, ROW_H);

    let build = |item: Item, row: UiRect| match item {
        Item::Folder {
            key,
            name,
            depth,
            expanded,
            count,
        } => folder_row(row, key, name, depth, expanded, count, state),
        Item::File {
            name,
            kind,
            path,
            depth,
        } => file_row(
            row,
            oid,
            kind,
            &name,
            &path,
            depth,
            repository,
            state,
            editor_focus,
        ),
    };
    let rect_at = |top: f32| UiRect::new(body.left + 8.0, top, body.right - 8.0, top + ROW_H);
    let mut content = Vec::new();
    for (index, item) in rows.iter().enumerate() {
        let top = tops[index];
        if top + ROW_H < view_top
            || top > view_bottom
            || pinned.iter().any(|(pinned, _)| *pinned == index)
        {
            continue;
        }
        content.push(build(item.clone(), rect_at(top)));
    }
    // Drawn last and opaque, so what scrolls beneath cannot show through.
    for (index, top) in pinned {
        let slot = rect_at(top);
        content.push(
            panel(
                UiRect::new(body.left, slot.top, body.right, slot.bottom),
                VisualStyle::filled(theme::c().sidebar),
            )
            .child(build(rows[index].clone(), slot)),
        );
    }
    scroll_region(
        body,
        content,
        content_bottom,
        scroll,
        max_scroll,
        state.clone(),
        ScrollRegion::CommitFiles,
    )
}

/// A folder of the file tree: click to fold or unfold it.
fn folder_row(
    rect: UiRect,
    key: String,
    name: String,
    depth: usize,
    expanded: bool,
    count: usize,
    state: &Binding,
) -> Element {
    let left = rect.left + depth as f32 * INDENT;
    let toggle = state.clone();
    let id = key.clone();
    let label = format!("{name}  {count}");
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
    .key(format!("git-history-detail-folder-{key}"))
    .event_policy(EventPolicy::INTERACTIVE)
    .semantics(Semantics::new(SemanticRole::Button).name(format!("{name} folder")))
    .cursor(CursorIcon::Pointer)
    .on_click(move || {
        toggle.update(|app| {
            if !app.ui.history_collapsed.remove(&id) {
                app.ui.history_collapsed.insert(id.clone());
            }
        });
    })
    .child(crate::ui::drawer::drawer_icon(
        UiId::owned(format!("git-history-detail-folder-icon-{key}")),
        if expanded {
            "chevron-down"
        } else {
            "chevron-right"
        },
        theme::icon_rect(UiRect::new(
            left + 2.0,
            rect.top + 4.0,
            left + 14.0,
            rect.bottom - 4.0,
        )),
        theme::c().text_faint,
    ))
    .child(text(
        UiRect::new(left + 18.0, rect.top, rect.right - 4.0, rect.bottom),
        tabs::ellipsize(&label, rect.right - left - 22.0, theme::SMALL, 400),
        theme::mono(theme::c().text_muted, theme::SMALL),
    ))
}

fn file_row(
    rect: UiRect,
    oid: &str,
    kind: ChangeKind,
    name: &str,
    path: &Path,
    depth: usize,
    repository: &Path,
    state: &Binding,
    editor_focus: &UiFocusHandle,
) -> Element {
    let color = change_color(kind);
    let (state, focus) = (state.clone(), editor_focus.clone());
    let (root, oid_owned, open_path) =
        (repository.to_path_buf(), oid.to_owned(), path.to_path_buf());
    let key = format!("git-history-detail-{oid}-{}", path.display());
    let label = path.to_path_buf();
    let indent = depth as f32 * INDENT;
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
        UiRect::new(
            rect.left + indent + 6.0,
            rect.top,
            rect.left + indent + 18.0,
            rect.bottom,
        ),
        kind.indicator(),
        theme::mono_bold(color, theme::SMALL),
    ))
    .child(change_path_label(
        UiRect::new(
            rect.left + indent + 22.0,
            rect.top,
            rect.right - 6.0,
            rect.bottom,
        ),
        Path::new(name),
        false,
        kind,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn shape(items: &[Item]) -> Vec<String> {
        items
            .iter()
            .map(|item| match item {
                Item::Folder {
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
                Item::File { name, depth, .. } => format!("{}{name}", " ".repeat(*depth)),
            })
            .collect()
    }

    fn changed(paths: &[&str]) -> Vec<(ChangeKind, PathBuf)> {
        paths
            .iter()
            .map(|path| (ChangeKind::Modified, PathBuf::from(path)))
            .collect()
    }

    #[test]
    fn changed_files_form_a_tree_with_single_child_folders_joined() {
        let files = changed(&[
            "scripts/architecture/policy.json",
            "src/app/git.rs",
            "src/app/bootstrap.rs",
            "src/ui/git/drawer.rs",
            "README.md",
        ]);
        assert_eq!(
            shape(&items(&files, &HashSet::new())),
            [
                "scripts/architecture/ 1",
                " policy.json",
                "src/ 3",
                " app/ 2",
                "  bootstrap.rs",
                "  git.rs",
                " ui/git/ 1",
                "  drawer.rs",
                "README.md",
            ]
        );
    }

    #[test]
    fn the_header_shows_the_whole_commit_and_leaves_room_for_files() {
        let commit = CommitSummary {
            oid: "0123456789abcdef".into(),
            parents: vec!["fedcba9876543210".into()],
            author: "Ada".into(),
            author_email: "ada@example.com".into(),
            timestamp: 0,
            subject: "feat: subject".into(),
            decorations: vec!["HEAD -> refs/heads/main".into()],
        };
        let details = Ok(CommitDetails {
            author: "Ada".into(),
            author_email: "ada@example.com".into(),
            date: "2026-01-02 03:04".into(),
            message: "feat: subject

A body line.
Another body line."
                .into(),
            files_changed: 2,
            insertions: 5,
            deletions: 1,
        });
        let subject = Subject::Commit(&commit);
        let tall = UiRect::new(0.0, 0.0, 253.0, 600.0);
        let short = UiRect::new(0.0, 0.0, 253.0, 300.0);
        let text_of = |layout: &Layout| -> Vec<String> {
            layout
                .lines
                .iter()
                .filter_map(|line| match line {
                    Line::Text(parts) => {
                        Some(parts.iter().map(|(text, _)| text.as_str()).collect())
                    }
                    Line::Gap => None,
                })
                .collect()
        };
        let full = layout(tall, subject, Some(&details));
        let lines = text_of(&full);
        assert!(lines.iter().any(|line| line.contains("A body line.")));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("Ada <ada@example.com>"))
        );
        assert!(lines.iter().any(|line| line.contains("2026-01-02 03:04")));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("2 files") && line.contains("+5"))
        );
        assert!(lines.iter().any(|line| line.contains("main")));
        assert!(lines.iter().any(|line| line.starts_with("parent fedcba9")));
        // A short pane trims the message, never the file tree.
        let cramped = layout(short, subject, Some(&details));
        assert!(cramped.files_top + MIN_FILES_H <= short.height() + 40.0);
        assert!(cramped.files_top < full.files_top);
        // Before the details arrive the header says so.
        let waiting = text_of(&layout(tall, subject, None));
        assert!(waiting.iter().any(|line| line.contains("Loading details")));
    }

    #[test]
    fn folded_folders_hide_their_files() {
        let files = changed(&["src/app/git.rs", "src/app/shell.rs"]);
        let collapsed = HashSet::from(["f:src/app".to_owned()]);
        assert_eq!(shape(&items(&files, &collapsed)), ["src/app/ 2 (folded)"]);
    }
}
