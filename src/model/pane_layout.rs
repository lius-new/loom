//! Split tree of editor panes.
//!
//! Mirrors VSCode's gridview: a split's children run along its axis, nested
//! splits always alternate axes, a split never keeps a single child, and
//! sizes are stored as fractions so the tree scales with the window.

use lgui::prelude::UiRect;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PaneId(u64);

impl PaneId {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Direction in which a split lays out its children.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    /// Children side by side, left to right.
    Horizontal,
    /// Children stacked, top to bottom.
    Vertical,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

impl Direction {
    pub fn axis(self) -> Axis {
        match self {
            Self::Left | Self::Right => Axis::Horizontal,
            Self::Up | Self::Down => Axis::Vertical,
        }
    }

    /// Whether the new pane goes after (right of / below) the target.
    fn after(self) -> bool {
        matches!(self, Self::Right | Self::Down)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Child {
    pub node: PaneNode,
    /// Fraction of the parent's extent along its axis; siblings sum to 1.
    pub size: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PaneNode {
    Leaf(PaneId),
    Split { axis: Axis, children: Vec<Child> },
}

/// A draggable boundary between two adjacent children of one split.
#[derive(Clone, Debug, PartialEq)]
pub struct Sash {
    /// Child indices from the root to the split that owns this boundary.
    pub path: Vec<usize>,
    /// The boundary lies between `children[index]` and `children[index + 1]`.
    pub index: usize,
    pub axis: Axis,
    /// The split's own rectangle; `resize` positions are relative to it.
    pub parent: UiRect,
    /// Boundary position along `axis`, in absolute coordinates.
    pub position: f32,
    /// Extent of the boundary across the axis: (start, end).
    pub span: (f32, f32),
}

/// Smallest width and height a pane may be resized to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MinSize {
    pub width: f32,
    pub height: f32,
}

impl PaneNode {
    pub fn contains(&self, id: PaneId) -> bool {
        match self {
            Self::Leaf(leaf) => *leaf == id,
            Self::Split { children, .. } => children.iter().any(|child| child.node.contains(id)),
        }
    }

    /// Leaves in reading order (depth-first, left to right, top to bottom).
    pub fn leaves(&self) -> Vec<PaneId> {
        let mut leaves = Vec::new();
        self.collect_leaves(&mut leaves);
        leaves
    }

    fn collect_leaves(&self, out: &mut Vec<PaneId>) {
        match self {
            Self::Leaf(id) => out.push(*id),
            Self::Split { children, .. } => {
                for child in children {
                    child.node.collect_leaves(out);
                }
            }
        }
    }

    /// Place `new` next to `target` on the `direction` side, halving the
    /// target's space. Returns false when `target` is not in the tree.
    pub fn split(&mut self, target: PaneId, new: PaneId, direction: Direction) -> bool {
        if let Self::Leaf(id) = self {
            if *id != target {
                return false;
            }
            *self = pair(direction, target, new);
            return true;
        }
        let Self::Split { axis, children } = self else {
            unreachable!()
        };
        let Some(index) = children
            .iter()
            .position(|child| child.node.contains(target))
        else {
            return false;
        };
        if children[index].node != Self::Leaf(target) {
            return children[index].node.split(target, new, direction);
        }
        if *axis == direction.axis() {
            let half = children[index].size / 2.0;
            children[index].size = half;
            let at = if direction.after() { index + 1 } else { index };
            children.insert(
                at,
                Child {
                    node: Self::Leaf(new),
                    size: half,
                },
            );
        } else {
            children[index].node = pair(direction, target, new);
        }
        true
    }

    /// Remove a pane, giving its space to the adjacent sibling and collapsing
    /// splits left with one child. The last pane cannot be removed.
    pub fn remove(&mut self, target: PaneId) -> bool {
        let removed = match self {
            Self::Leaf(_) => false,
            Self::Split { children, .. } => remove_from(children, target),
        };
        if removed {
            self.normalize();
        }
        removed
    }

    /// Restore the tree invariants: no single-child splits, no split nested
    /// directly in a split of the same axis, and sizes summing to 1.
    pub fn normalize(&mut self) {
        let Self::Split { axis, children } = self else {
            return;
        };
        for child in children.iter_mut() {
            child.node.normalize();
        }
        let axis = *axis;
        let mut flattened = Vec::with_capacity(children.len());
        for child in children.drain(..) {
            match child.node {
                Self::Split {
                    axis: inner,
                    children: grandchildren,
                } if inner == axis => {
                    flattened.extend(grandchildren.into_iter().map(|grandchild| Child {
                        node: grandchild.node,
                        size: grandchild.size * child.size,
                    }));
                }
                node => flattened.push(Child {
                    node,
                    size: child.size,
                }),
            }
        }
        if flattened.len() == 1 {
            *self = flattened.pop().unwrap().node;
            return;
        }
        let total: f32 = flattened.iter().map(|child| child.size).sum();
        for child in &mut flattened {
            child.size = if total > f32::EPSILON {
                child.size / total
            } else {
                1.0
            };
        }
        if total <= f32::EPSILON {
            let even = 1.0 / flattened.len() as f32;
            flattened.iter_mut().for_each(|child| child.size = even);
        }
        *self = Self::Split {
            axis,
            children: flattened,
        };
    }

    /// Rectangles of every pane inside `rect`. Boundaries are rounded to whole
    /// pixels so adjacent panes neither overlap nor leave a gap.
    pub fn layout(&self, rect: UiRect) -> Vec<(PaneId, UiRect)> {
        let mut out = Vec::new();
        self.layout_into(rect, &mut out);
        out
    }

    fn layout_into(&self, rect: UiRect, out: &mut Vec<(PaneId, UiRect)>) {
        match self {
            Self::Leaf(id) => out.push((*id, rect)),
            Self::Split { axis, children } => {
                for (child, child_rect) in children.iter().zip(child_rects(*axis, children, rect)) {
                    child.node.layout_into(child_rect, out);
                }
            }
        }
    }

    pub fn sashes(&self, rect: UiRect) -> Vec<Sash> {
        let mut out = Vec::new();
        self.sashes_into(rect, &mut Vec::new(), &mut out);
        out
    }

    fn sashes_into(&self, rect: UiRect, path: &mut Vec<usize>, out: &mut Vec<Sash>) {
        let Self::Split { axis, children } = self else {
            return;
        };
        let rects = child_rects(*axis, children, rect);
        for (index, child_rect) in rects.iter().enumerate() {
            if index + 1 < rects.len() {
                let (position, span) = match axis {
                    Axis::Horizontal => (child_rect.right, (rect.top, rect.bottom)),
                    Axis::Vertical => (child_rect.bottom, (rect.left, rect.right)),
                };
                out.push(Sash {
                    path: path.clone(),
                    index,
                    axis: *axis,
                    parent: rect,
                    position,
                    span,
                });
            }
            path.push(index);
            children[index].node.sashes_into(*child_rect, path, out);
            path.pop();
        }
    }

    /// Move one sash to `position` (absolute, along the sash's axis), keeping
    /// both neighbours at least as large as their subtrees allow.
    pub fn resize(&mut self, sash: &Sash, position: f32, min: MinSize) -> bool {
        let Some(Self::Split { axis, children }) = self.node_at_mut(&sash.path) else {
            return false;
        };
        let index = sash.index;
        if index + 1 >= children.len() {
            return false;
        }
        let (start, extent) = match axis {
            Axis::Horizontal => (sash.parent.left, sash.parent.width()),
            Axis::Vertical => (sash.parent.top, sash.parent.height()),
        };
        if extent <= f32::EPSILON {
            return false;
        }
        let before: f32 = children[..index].iter().map(|child| child.size).sum();
        let pair = children[index].size + children[index + 1].size;
        let min_a = children[index].node.min_extent(*axis, min) / extent;
        let min_b = children[index + 1].node.min_extent(*axis, min) / extent;
        let fraction = (position - start) / extent - before;
        let first = if min_a + min_b >= pair {
            pair * min_a / (min_a + min_b).max(f32::EPSILON)
        } else {
            fraction.clamp(min_a, pair - min_b)
        };
        if (children[index].size - first).abs() <= f32::EPSILON {
            return false;
        }
        children[index].size = first;
        children[index + 1].size = pair - first;
        true
    }

    /// Give every child of every split an equal share.
    pub fn even(&mut self) {
        if let Self::Split { children, .. } = self {
            let size = 1.0 / children.len() as f32;
            for child in children {
                child.size = size;
                child.node.even();
            }
        }
    }

    /// Panes adjacent to `from` on the `direction` side, best overlap first.
    pub fn neighbors(&self, rect: UiRect, from: PaneId, direction: Direction) -> Vec<PaneId> {
        const TOUCH: f32 = 1.0;
        let rects = self.layout(rect);
        let Some(origin) = rects.iter().find(|(id, _)| *id == from).map(|(_, r)| *r) else {
            return Vec::new();
        };
        let mut found = rects
            .iter()
            .filter(|(id, _)| *id != from)
            .filter_map(|(id, r)| {
                let (touches, overlap) = match direction {
                    Direction::Left => (
                        (r.right - origin.left).abs() <= TOUCH,
                        overlap(r.top, r.bottom, origin.top, origin.bottom),
                    ),
                    Direction::Right => (
                        (r.left - origin.right).abs() <= TOUCH,
                        overlap(r.top, r.bottom, origin.top, origin.bottom),
                    ),
                    Direction::Up => (
                        (r.bottom - origin.top).abs() <= TOUCH,
                        overlap(r.left, r.right, origin.left, origin.right),
                    ),
                    Direction::Down => (
                        (r.top - origin.bottom).abs() <= TOUCH,
                        overlap(r.left, r.right, origin.left, origin.right),
                    ),
                };
                (touches && overlap > 0.0).then_some((*id, overlap))
            })
            .collect::<Vec<_>>();
        found.sort_by(|a, b| b.1.total_cmp(&a.1));
        found.into_iter().map(|(id, _)| id).collect()
    }

    fn node_at_mut(&mut self, path: &[usize]) -> Option<&mut PaneNode> {
        let Some((first, rest)) = path.split_first() else {
            return Some(self);
        };
        match self {
            Self::Leaf(_) => None,
            Self::Split { children, .. } => children.get_mut(*first)?.node.node_at_mut(rest),
        }
    }

    /// Minimum extent of this subtree along `axis`.
    fn min_extent(&self, axis: Axis, min: MinSize) -> f32 {
        match self {
            Self::Leaf(_) => match axis {
                Axis::Horizontal => min.width,
                Axis::Vertical => min.height,
            },
            Self::Split {
                axis: inner,
                children,
            } => {
                let extents = children
                    .iter()
                    .map(|child| child.node.min_extent(axis, min));
                if *inner == axis {
                    extents.sum()
                } else {
                    extents.fold(0.0, f32::max)
                }
            }
        }
    }
}

fn pair(direction: Direction, target: PaneId, new: PaneId) -> PaneNode {
    let (first, second) = if direction.after() {
        (target, new)
    } else {
        (new, target)
    };
    PaneNode::Split {
        axis: direction.axis(),
        children: vec![
            Child {
                node: PaneNode::Leaf(first),
                size: 0.5,
            },
            Child {
                node: PaneNode::Leaf(second),
                size: 0.5,
            },
        ],
    }
}

fn remove_from(children: &mut Vec<Child>, target: PaneId) -> bool {
    if let Some(index) = children
        .iter()
        .position(|child| child.node == PaneNode::Leaf(target))
    {
        let freed = children.remove(index).size;
        let heir = if index > 0 { index - 1 } else { 0 };
        if let Some(child) = children.get_mut(heir) {
            child.size += freed;
        }
        return true;
    }
    children.iter_mut().any(|child| match &mut child.node {
        PaneNode::Split { children, .. } => remove_from(children, target),
        PaneNode::Leaf(_) => false,
    })
}

fn child_rects(axis: Axis, children: &[Child], rect: UiRect) -> Vec<UiRect> {
    let (start, extent) = match axis {
        Axis::Horizontal => (rect.left, rect.width()),
        Axis::Vertical => (rect.top, rect.height()),
    };
    let end = start + extent;
    let mut offset = 0.0;
    let mut edge = start;
    children
        .iter()
        .enumerate()
        .map(|(index, child)| {
            offset += child.size;
            let next = if index + 1 == children.len() {
                end
            } else {
                (start + offset * extent).round().clamp(edge, end)
            };
            let child_rect = match axis {
                Axis::Horizontal => UiRect::new(edge, rect.top, next, rect.bottom),
                Axis::Vertical => UiRect::new(rect.left, edge, rect.right, next),
            };
            edge = next;
            child_rect
        })
        .collect()
}

fn overlap(a0: f32, a1: f32, b0: f32, b1: f32) -> f32 {
    (a1.min(b1) - a0.max(b0)).max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: PaneId = PaneId::new(1);
    const B: PaneId = PaneId::new(2);
    const C: PaneId = PaneId::new(3);
    const D: PaneId = PaneId::new(4);
    const AREA: UiRect = UiRect::new(0.0, 0.0, 800.0, 600.0);
    const MIN: MinSize = MinSize {
        width: 100.0,
        height: 100.0,
    };

    fn rect_of(node: &PaneNode, id: PaneId) -> UiRect {
        node.layout(AREA)
            .into_iter()
            .find(|(pane, _)| *pane == id)
            .unwrap()
            .1
    }

    fn assert_invariants(node: &PaneNode, parent: Option<Axis>) {
        if let PaneNode::Split { axis, children } = node {
            assert!(children.len() >= 2, "split with one child: {node:?}");
            assert_ne!(Some(*axis), parent, "nested split of the same axis");
            let total: f32 = children.iter().map(|child| child.size).sum();
            assert!((total - 1.0).abs() < 1e-4, "sizes sum to {total}");
            for child in children {
                assert_invariants(&child.node, Some(*axis));
            }
        }
    }

    #[test]
    fn parallel_split_inserts_a_sibling_and_orthogonal_split_nests() {
        let mut tree = PaneNode::Leaf(A);
        assert!(tree.split(A, B, Direction::Right));
        assert!(tree.split(B, C, Direction::Right));
        assert_eq!(tree.leaves(), vec![A, B, C]);
        let PaneNode::Split { axis, children } = &tree else {
            panic!()
        };
        assert_eq!(*axis, Axis::Horizontal);
        assert_eq!(children.len(), 3);
        assert_eq!(
            children.iter().map(|c| c.size).collect::<Vec<_>>(),
            vec![0.5, 0.25, 0.25]
        );

        assert!(tree.split(B, D, Direction::Up));
        assert_eq!(tree.leaves(), vec![A, D, B, C]);
        assert_eq!(rect_of(&tree, D), UiRect::new(400.0, 0.0, 600.0, 300.0));
        assert_eq!(rect_of(&tree, B), UiRect::new(400.0, 300.0, 600.0, 600.0));
        assert_invariants(&tree, None);
        assert!(!tree.split(PaneId::new(99), D, Direction::Left));
    }

    #[test]
    fn removing_collapses_single_child_splits_into_the_grandparent() {
        let mut tree = PaneNode::Leaf(A);
        tree.split(A, B, Direction::Right);
        tree.split(B, C, Direction::Down);
        tree.split(C, D, Direction::Right);
        assert_invariants(&tree, None);

        assert!(tree.remove(B));
        // B's column held only C|D after removal; that row merges into the root row.
        assert_invariants(&tree, None);
        assert_eq!(tree.leaves(), vec![A, C, D]);
        let PaneNode::Split { axis, children } = &tree else {
            panic!()
        };
        assert_eq!(*axis, Axis::Horizontal);
        assert_eq!(children.len(), 3);

        assert!(tree.remove(C));
        assert!(tree.remove(D));
        assert_eq!(tree, PaneNode::Leaf(A));
        assert!(!tree.remove(A));
    }

    #[test]
    fn removed_space_goes_to_the_previous_sibling() {
        let mut tree = PaneNode::Leaf(A);
        tree.split(A, B, Direction::Right);
        tree.split(B, C, Direction::Right);
        tree.remove(B);
        assert_eq!(rect_of(&tree, A).width(), 600.0);
        assert_eq!(rect_of(&tree, C).width(), 200.0);
    }

    #[test]
    fn layout_rounds_boundaries_without_gaps() {
        let mut tree = PaneNode::Leaf(A);
        tree.split(A, B, Direction::Right);
        tree.split(B, C, Direction::Right);
        let area = UiRect::new(10.0, 0.0, 311.0, 100.0);
        let rects = tree.layout(area);
        assert_eq!(rects[0].1.left, 10.0);
        assert_eq!(rects[0].1.right, rects[1].1.left);
        assert_eq!(rects[1].1.right, rects[2].1.left);
        assert_eq!(rects[2].1.right, 311.0);
        assert!(rects.iter().all(|(_, r)| r.left.fract() == 0.0));
    }

    #[test]
    fn resize_moves_a_sash_and_respects_minimum_sizes() {
        let mut tree = PaneNode::Leaf(A);
        tree.split(A, B, Direction::Right);
        tree.split(B, C, Direction::Down);
        let sashes = tree.sashes(AREA);
        assert_eq!(sashes.len(), 2);
        let column = sashes.iter().find(|s| s.axis == Axis::Horizontal).unwrap();
        assert_eq!(column.position, 400.0);
        assert_eq!(column.span, (0.0, 600.0));

        assert!(tree.resize(column, 300.0, MIN));
        assert_eq!(rect_of(&tree, A).right, 300.0);
        tree.resize(column, 20.0, MIN);
        assert_eq!(rect_of(&tree, A).right, 100.0);
        tree.resize(column, 790.0, MIN);
        assert_eq!(rect_of(&tree, B).width(), 100.0);

        let row = tree
            .sashes(AREA)
            .into_iter()
            .find(|s| s.axis == Axis::Vertical)
            .unwrap();
        assert_eq!(row.path, vec![1]);
        assert!(tree.resize(&row, 450.0, MIN));
        assert_eq!(rect_of(&tree, B).bottom, 450.0);
        assert_invariants(&tree, None);
    }

    #[test]
    fn nested_subtrees_contribute_their_minimum_extent() {
        let mut tree = PaneNode::Leaf(A);
        tree.split(A, B, Direction::Right);
        tree.split(B, C, Direction::Down);
        tree.split(C, D, Direction::Right);
        let column = tree
            .sashes(AREA)
            .into_iter()
            .find(|s| s.path.is_empty())
            .unwrap();
        tree.resize(&column, 700.0, MIN);
        // The right column contains C|D side by side, so it needs 200px.
        assert_eq!(rect_of(&tree, A).right, 600.0);
    }

    #[test]
    fn neighbors_follow_geometry() {
        let mut tree = PaneNode::Leaf(A);
        tree.split(A, B, Direction::Right);
        tree.split(B, C, Direction::Down);
        assert_eq!(tree.neighbors(AREA, A, Direction::Right), vec![B, C]);
        assert_eq!(tree.neighbors(AREA, C, Direction::Left), vec![A]);
        assert_eq!(tree.neighbors(AREA, C, Direction::Up), vec![B]);
        assert!(tree.neighbors(AREA, A, Direction::Left).is_empty());
    }

    #[test]
    fn even_resets_all_sizes() {
        let mut tree = PaneNode::Leaf(A);
        tree.split(A, B, Direction::Right);
        tree.split(B, C, Direction::Right);
        tree.even();
        let widths = tree
            .layout(UiRect::new(0.0, 0.0, 900.0, 10.0))
            .iter()
            .map(|(_, r)| r.width())
            .collect::<Vec<_>>();
        assert_eq!(widths, vec![300.0, 300.0, 300.0]);
    }
}
