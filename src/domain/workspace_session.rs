//! Stable editor session DTOs and pure snapshot/shape conversions.
use crate::domain::identity::{FileId, PaneId};
use crate::domain::pane_layout::{Axis, Child, PaneNode};
use crate::domain::workspace::Workspace;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SavedAxis {
    Horizontal,
    Vertical,
}

/// One node of the saved pane tree.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SavedPaneNode {
    Pane {
        items: Vec<PathBuf>,
        #[serde(default)]
        active: Option<PathBuf>,
    },
    Split {
        axis: SavedAxis,
        children: Vec<SavedChild>,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SavedChild {
    /// Fraction of the parent split.
    pub size: f32,
    pub node: SavedPaneNode,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct EditorLayout {
    pub root: SavedPaneNode,
    /// Index of the focused pane in reading order.
    #[serde(default)]
    pub active_pane: usize,
}

pub struct PendingTab {
    pub pane: PaneId,
    pub path: PathBuf,
    pub insert_before: Option<usize>,
}

pub fn capture<D: Clone, K: Clone + Ord>(
    workspace: &Workspace<D, K>,
    pending: Option<&PendingTab>,
) -> Option<EditorLayout> {
    let mut leaf_index = 0;
    let mut active_pane = None;
    let root = save_pane_node(
        workspace,
        workspace.layout(),
        pending,
        &mut leaf_index,
        &mut active_pane,
    )?;
    Some(EditorLayout {
        root,
        active_pane: active_pane.unwrap_or(0),
    })
}

fn save_pane_node<D: Clone, K: Clone + Ord>(
    workspace: &Workspace<D, K>,
    node: &PaneNode,
    pending: Option<&PendingTab>,
    leaf_index: &mut usize,
    active_pane: &mut Option<usize>,
) -> Option<SavedPaneNode> {
    match node {
        PaneNode::Leaf(id) => {
            let pane = workspace.pane(*id)?;
            let restorable = |file: FileId| workspace.is_file(file);
            let path = |file: FileId| workspace.meta(file).map(|meta| meta.path.clone());
            let mut items = pane
                .items()
                .iter()
                .copied()
                .filter(|file| restorable(*file))
                .filter_map(path)
                .collect::<Vec<_>>();
            let pending = pending.filter(|p| p.pane == *id);
            if let Some(pending) = pending
                && !items.contains(&pending.path)
            {
                let index = if let Some(index) = pending.insert_before {
                    pane.items()
                        .iter()
                        .take(index)
                        .filter(|file| workspace.is_file(**file))
                        .count()
                } else {
                    items.len()
                };
                items.insert(index.min(items.len()), pending.path.clone());
            }
            if items.is_empty() {
                return None;
            }
            if pending.is_some() || (active_pane.is_none() && *id == workspace.active_pane()) {
                *active_pane = Some(*leaf_index);
            }
            *leaf_index += 1;
            let active = pending.map(|p| p.path.clone()).or_else(|| {
                pane.active()
                    .filter(|file| restorable(*file))
                    .and_then(path)
            });
            Some(SavedPaneNode::Pane { items, active })
        }
        PaneNode::Split { axis, children } => {
            let mut saved = children
                .iter()
                .filter_map(|child| {
                    save_pane_node(workspace, &child.node, pending, leaf_index, active_pane).map(
                        |node| SavedChild {
                            size: child.size,
                            node,
                        },
                    )
                })
                .collect::<Vec<_>>();
            match saved.len() {
                0 => None,
                1 => saved.pop().map(|child| child.node),
                _ => {
                    let total: f32 = saved.iter().map(|child| child.size).sum();
                    for child in &mut saved {
                        child.size /= total.max(f32::EPSILON);
                    }
                    Some(SavedPaneNode::Split {
                        axis: match axis {
                            Axis::Horizontal => SavedAxis::Horizontal,
                            Axis::Vertical => SavedAxis::Vertical,
                        },
                        children: saved,
                    })
                }
            }
        }
    }
}

pub fn restored_shape(
    node: SavedPaneNode,
    contents: &mut Vec<(Vec<PathBuf>, Option<PathBuf>)>,
) -> PaneNode {
    match node {
        SavedPaneNode::Pane { items, active } => {
            let leaf = PaneId::new(contents.len() as u64);
            contents.push((items, active));
            PaneNode::Leaf(leaf)
        }
        SavedPaneNode::Split { axis, children } if !children.is_empty() => PaneNode::Split {
            axis: match axis {
                SavedAxis::Horizontal => Axis::Horizontal,
                SavedAxis::Vertical => Axis::Vertical,
            },
            children: children
                .into_iter()
                .map(|child| Child {
                    size: child.size,
                    node: restored_shape(child.node, contents),
                })
                .collect(),
        },
        SavedPaneNode::Split { .. } => {
            let leaf = PaneId::new(contents.len() as u64);
            contents.push((Vec::new(), None));
            PaneNode::Leaf(leaf)
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::domain::document::DiskState;
    use crate::domain::pane_layout::Direction;
    use crate::domain::workspace::{AppPage, OpenMode, PreparedTextFile, Workspace};
    use crate::domain::workspace_session::{self, PendingTab, SavedPaneNode};

    fn open(workspace: &mut Workspace<(), ()>, path: &str) {
        workspace.open_prepared_text(
            PreparedTextFile::with_disk_state(
                path.into(),
                "text".into(),
                DiskState::from_contents("text"),
            ),
            OpenMode::Permanent,
        );
    }

    #[test]
    fn pending_snapshot_counts_only_file_tabs_before_its_insertion() {
        let mut workspace = Workspace::<(), ()>::new();
        open(&mut workspace, "first.txt");
        workspace.open_page(AppPage::Settings);
        open(&mut workspace, "second.txt");
        let pending = PendingTab {
            pane: workspace.active_pane(),
            path: "queued.txt".into(),
            insert_before: Some(2),
        };
        let layout = workspace_session::capture(&workspace, Some(&pending)).unwrap();
        let SavedPaneNode::Pane { items, active } = layout.root else {
            panic!("one pane");
        };
        assert_eq!(
            items,
            [
                std::path::PathBuf::from("first.txt"),
                "queued.txt".into(),
                "second.txt".into()
            ]
        );
        assert_eq!(active, Some("queued.txt".into()));
        assert_eq!(layout.active_pane, 0);
        assert_eq!(
            workspace.active_meta().unwrap().path,
            std::path::Path::new("second.txt")
        );
    }

    #[test]
    fn pruning_empty_panes_preserves_focus_and_restorable_tree_order() {
        let mut workspace = Workspace::<(), ()>::new();
        open(&mut workspace, "first.txt");
        let left = workspace.active_pane();
        let empty = workspace.split_empty(left, Direction::Right).unwrap();
        let last = workspace.split_empty(empty, Direction::Down).unwrap();
        workspace.activate_pane(last);
        open(&mut workspace, "last.txt");
        let layout = workspace_session::capture(&workspace, None).unwrap();
        assert_eq!(layout.active_pane, 1);
        let encoded = serde_json::to_string(&layout).unwrap();
        let decoded: workspace_session::EditorLayout = serde_json::from_str(&encoded).unwrap();
        let mut tabs = Vec::new();
        let tree = workspace_session::restored_shape(decoded.root, &mut tabs);
        assert_eq!(tree.leaves().len(), 2);
        assert_eq!(tabs[0].0, [std::path::PathBuf::from("first.txt")]);
        assert_eq!(tabs[1].0, [std::path::PathBuf::from("last.txt")]);
        let mut restored = Workspace::<(), ()>::new();
        assert_eq!(restored.restore_layout(tree).len(), 2);
    }
}
