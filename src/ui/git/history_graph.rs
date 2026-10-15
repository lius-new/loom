//! Parent-based commit lanes, including continuations through expanded files.

use lgui::core::{
    PathStyle, Point, Stroke, UiElement, UiId, UiPath, UiPathCommand, path, precompiled,
};
use lgui::prelude::{Color, Element, UiRect, VisualStyle, group};

use super::panel::HistoryRow;
use crate::ui::theme;

pub(crate) struct RenderedRow {
    pub element: Element,
    pub width: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Track {
    lane: usize,
    color: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Edge {
    from: usize,
    to: usize,
    color: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Row {
    node: Option<Track>,
    above: Vec<Track>,
    below: Vec<Track>,
    edges: Vec<Edge>,
}

struct Pending<'a> {
    oid: &'a str,
    color: usize,
}

pub(crate) struct Graph {
    rows: Vec<Row>,
    lanes: usize,
}

fn tracks(pending: &[Option<Pending<'_>>]) -> Vec<Track> {
    pending
        .iter()
        .enumerate()
        .filter_map(|(lane, target)| {
            target.as_ref().map(|target| Track {
                lane,
                color: target.color,
            })
        })
        .collect()
}

fn vacant(pending: &mut Vec<Option<Pending<'_>>>) -> usize {
    if let Some(lane) = pending.iter().position(Option::is_none) {
        lane
    } else {
        pending.push(None);
        pending.len() - 1
    }
}

impl Graph {
    pub(crate) fn new(history: &[HistoryRow]) -> Self {
        let mut pending: Vec<Option<Pending<'_>>> = Vec::new();
        let mut next_color = 0;
        let mut rows = Vec::with_capacity(history.len());
        let mut lanes = 1;
        for entry in history {
            let above = tracks(&pending);
            let HistoryRow::Commit(commit, _) = entry else {
                rows.push(Row {
                    node: None,
                    edges: above
                        .iter()
                        .map(|track| Edge {
                            from: track.lane,
                            to: track.lane,
                            color: track.color,
                        })
                        .collect(),
                    below: above.clone(),
                    above,
                });
                continue;
            };
            let lane = pending
                .iter()
                .position(|target| {
                    target
                        .as_ref()
                        .is_some_and(|target| target.oid == commit.oid)
                })
                .unwrap_or_else(|| {
                    let lane = vacant(&mut pending);
                    pending[lane] = Some(Pending {
                        oid: &commit.oid,
                        color: next_color,
                    });
                    next_color += 1;
                    lane
                });
            let node = Track {
                lane,
                color: pending[lane].take().unwrap().color,
            };
            let mut edges = above
                .iter()
                .filter(|track| track.lane != lane)
                .map(|track| Edge {
                    from: track.lane,
                    to: track.lane,
                    color: track.color,
                })
                .collect::<Vec<_>>();
            for (index, parent) in commit.parents.iter().enumerate() {
                if commit.parents[..index].contains(parent) {
                    continue;
                }
                let parent_lane = if let Some(existing) = pending
                    .iter()
                    .position(|target| target.as_ref().is_some_and(|target| target.oid == parent))
                {
                    existing
                } else {
                    let target_lane = if index == 0 && pending[lane].is_none() {
                        lane
                    } else {
                        vacant(&mut pending)
                    };
                    let color = if index == 0 {
                        node.color
                    } else {
                        let color = next_color;
                        next_color += 1;
                        color
                    };
                    pending[target_lane] = Some(Pending { oid: parent, color });
                    target_lane
                };
                edges.push(Edge {
                    from: lane,
                    to: parent_lane,
                    color: if index == 0 {
                        node.color
                    } else {
                        pending[parent_lane].as_ref().unwrap().color
                    },
                });
            }
            lanes = lanes.max(pending.len()).max(lane + 1);
            rows.push(Row {
                node: Some(node),
                above,
                below: tracks(&pending),
                edges,
            });
            // Keep interior holes stable; only vacant rightmost lanes disappear.
            while pending.last().is_some_and(Option::is_none) {
                pending.pop();
            }
        }
        Self { rows, lanes }
    }

    pub(crate) fn render(
        &self,
        index: usize,
        rect: UiRect,
        entry: &HistoryRow,
        hovered: bool,
    ) -> RenderedRow {
        let row = &self.rows[index];
        let width = 20.0 + self.lanes.saturating_sub(1).min(2) as f32 * 10.0;
        let gutter = UiRect::new(rect.left, rect.top, rect.left + width, rect.bottom);
        let mut graph = group(gutter).key("commit-graph");
        let spacing = (24.0 / self.lanes.saturating_sub(1).max(1) as f32).min(10.0);
        let x = |lane| rect.left + 12.0 + lane as f32 * spacing;
        let middle = (rect.top + rect.bottom) / 2.0;
        for track in &row.above {
            graph = graph.child(
                path(
                    gutter,
                    UiPath::new([
                        UiPathCommand::MoveTo(Point::new(x(track.lane), rect.top)),
                        UiPathCommand::LineTo(Point::new(x(track.lane), middle)),
                    ]),
                    line_style(track.color),
                )
                .key(format!("graph-above-{}", track.lane)),
            );
        }
        for (edge_index, edge) in row.edges.iter().enumerate() {
            graph = graph.child(
                path(
                    gutter,
                    UiPath::new([
                        UiPathCommand::MoveTo(Point::new(x(edge.from), middle)),
                        UiPathCommand::CubicTo {
                            control1: Point::new(x(edge.from), middle + rect.height() * 0.3),
                            control2: Point::new(x(edge.to), middle + rect.height() * 0.2),
                            to: Point::new(x(edge.to), rect.bottom),
                        },
                    ]),
                    line_style(edge.color),
                )
                .key(format!("graph-below-{edge_index}")),
            );
        }
        if let (Some(node), HistoryRow::Commit(commit, _)) = (row.node, entry) {
            let current = commit
                .decorations
                .iter()
                .any(|value| value == "HEAD" || value.starts_with("HEAD -> "));
            let radius = if current { 4.0_f32 } else { 3.0 }.min((spacing * 0.45).max(1.0));
            let node_rect = UiRect::new(
                x(node.lane) - radius,
                middle - radius,
                x(node.lane) + radius,
                middle + radius,
            );
            graph = graph.child(precompiled(UiElement::ellipse(
                UiId::owned(format!("git-history-node-{}", commit.oid)),
                node_rect,
                VisualStyle::filled(color(node.color)),
            )));
            if current || commit.parents.len() > 1 {
                let inset = radius * 0.5;
                graph = graph.child(precompiled(UiElement::ellipse(
                    UiId::owned(format!("git-history-node-center-{}", commit.oid)),
                    UiRect::new(
                        node_rect.left + inset,
                        node_rect.top + inset,
                        node_rect.right - inset,
                        node_rect.bottom - inset,
                    ),
                    VisualStyle::filled(if hovered {
                        theme::c().surface
                    } else {
                        theme::c().sidebar
                    }),
                )));
            }
        }
        RenderedRow {
            element: graph,
            width,
        }
    }
}

fn color(index: usize) -> Color {
    let colors = [
        theme::c().syntax.function,
        theme::c().syntax.property,
        theme::c().git.staged,
        theme::c().syntax.number,
        theme::c().syntax.keyword,
        theme::c().syntax.type_name,
    ];
    colors[index % colors.len()]
}

fn line_style(index: usize) -> PathStyle {
    PathStyle {
        fill: None,
        fill_alpha: 0,
        stroke: Some(Stroke::new(color(index), 1.5, 230)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::git::{ChangeKind, CommitSummary};

    fn commit(oid: &str, parents: &[&str]) -> HistoryRow {
        HistoryRow::Commit(
            CommitSummary {
                oid: oid.into(),
                parents: parents.iter().map(|parent| (*parent).into()).collect(),
                author: "author".into(),
                author_email: "author@example.com".into(),
                timestamp: 0,
                subject: oid.into(),
                decorations: Vec::new(),
            },
            false,
        )
    }

    #[test]
    fn linear_history_starts_at_the_tip_and_ends_at_the_root() {
        let history = [
            commit("new", &["middle"]),
            commit("middle", &["root"]),
            commit("root", &[]),
        ];
        let graph = Graph::new(&history);
        assert_eq!(graph.lanes, 1);
        assert!(graph.rows[0].above.is_empty());
        assert!(graph.rows[2].below.is_empty());
        assert!(graph.rows[2].edges.is_empty());
        for (index, row) in graph.rows.iter().enumerate() {
            assert_eq!(row.node, Some(Track { lane: 0, color: 0 }));
            if index < 2 {
                assert_eq!(
                    row.edges,
                    vec![Edge {
                        from: 0,
                        to: 0,
                        color: 0
                    }]
                );
                assert_eq!(row.below, graph.rows[index + 1].above);
            }
        }
    }

    #[test]
    fn a_merge_fans_out_to_both_parents_and_rejoins_the_shared_ancestor() {
        let history = [
            commit("merge", &["main", "topic"]),
            commit("main", &["base"]),
            commit("topic", &["base"]),
            commit("base", &[]),
        ];
        let graph = Graph::new(&history);
        assert_eq!(graph.lanes, 2);
        assert_eq!(
            graph.rows[0].edges,
            vec![
                Edge {
                    from: 0,
                    to: 0,
                    color: 0
                },
                Edge {
                    from: 0,
                    to: 1,
                    color: 1
                }
            ]
        );
        assert_eq!(graph.rows[1].node, Some(Track { lane: 0, color: 0 }));
        assert_eq!(graph.rows[2].node, Some(Track { lane: 1, color: 1 }));
        assert!(graph.rows[2].edges.contains(&Edge {
            from: 1,
            to: 0,
            color: 1
        }));
        assert_eq!(graph.rows[3].node, Some(Track { lane: 0, color: 0 }));
        assert!(graph.rows[3].below.is_empty());
    }

    #[test]
    fn octopus_merges_keep_pending_lanes_stable_when_another_lane_finishes() {
        let history = [
            commit("merge", &["a", "b", "c"]),
            commit("a", &["x"]),
            commit("b", &["x"]),
            commit("c", &["y"]),
            commit("x", &["root"]),
            commit("y", &["root"]),
            commit("root", &[]),
        ];
        let graph = Graph::new(&history);
        let lanes = graph
            .rows
            .iter()
            .map(|row| row.node.unwrap().lane)
            .collect::<Vec<_>>();
        assert_eq!(lanes, [0, 0, 1, 2, 0, 2, 0]);
        for pair in graph.rows.windows(2) {
            assert_eq!(pair[0].below, pair[1].above);
        }
        assert!(graph.rows.last().unwrap().below.is_empty());
    }

    #[test]
    fn expanded_files_and_later_pages_preserve_the_commit_graph_prefix() {
        let mut history = vec![
            commit("merge", &["main", "topic"]),
            HistoryRow::File {
                oid: "merge".into(),
                kind: ChangeKind::Modified,
                path: "src/a.rs".into(),
            },
            HistoryRow::Message("Loading files…".into()),
            commit("main", &["root"]),
        ];
        let first = Graph::new(&history);
        for row in &first.rows[1..3] {
            assert!(row.node.is_none());
            assert_eq!(row.above, first.rows[0].below);
            assert_eq!(row.below, row.above);
            assert!(row.edges.iter().all(|edge| edge.from == edge.to));
        }
        assert!(!first.rows.last().unwrap().below.is_empty());
        history.extend([commit("topic", &["root"]), commit("root", &[])]);
        let complete = Graph::new(&history);
        assert_eq!(first.rows, complete.rows[..first.rows.len()]);
        assert_eq!(first.lanes, complete.lanes);
        let collapsed = Graph::new(&[
            commit("merge", &["main", "topic"]),
            commit("main", &["root"]),
            commit("topic", &["root"]),
            commit("root", &[]),
        ]);
        assert_eq!(complete.rows[3], collapsed.rows[1]);
        assert_eq!(complete.rows[4], collapsed.rows[2]);
    }
}
