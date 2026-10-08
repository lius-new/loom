//! Window content topology. Sizes and terminal ownership are independent of PTYs.
use super::pane_layout::{Axis, Direction};
use lgui::prelude::UiRect;
use std::collections::{HashMap, HashSet};

pub type RegionId = u64;

#[derive(Clone, Debug, PartialEq)]
pub enum Content {
    Editor,
    Git,
    Files,
    /// Sessions share a tab strip; only the active tab occupies the body.
    Terminal {
        tabs: Vec<u64>,
        active: Option<u64>,
    },
}
#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Leaf {
        id: RegionId,
        content: Content,
    },
    Split {
        id: RegionId,
        axis: Axis,
        children: Vec<Node>,
    },
}
impl Node {
    pub fn id(&self) -> RegionId {
        match self {
            Self::Leaf { id, .. } | Self::Split { id, .. } => *id,
        }
    }
    fn find(&self, id: RegionId) -> Option<&Self> {
        if self.id() == id {
            return Some(self);
        }
        match self {
            Self::Split { children, .. } => children.iter().find_map(|n| n.find(id)),
            _ => None,
        }
    }
    fn find_mut(&mut self, id: RegionId) -> Option<&mut Self> {
        if self.id() == id {
            return Some(self);
        }
        match self {
            Self::Split { children, .. } => children.iter_mut().find_map(|n| n.find_mut(id)),
            _ => None,
        }
    }
    fn leaves(&self, out: &mut Vec<(RegionId, Content)>) {
        match self {
            Self::Leaf { id, content } => out.push((*id, content.clone())),
            Self::Split { children, .. } => {
                for n in children {
                    n.leaves(out);
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Visibility {
    pub git: bool,
    pub files: bool,
    pub terminal: bool,
}
#[derive(Clone, Debug, PartialEq)]
pub struct ApplicationLayout {
    pub root: Node,
    /// Fractions belong to split IDs, never to terminal sessions.
    pub sizes: HashMap<RegionId, Vec<f32>>,
    pub version: u64,
    /// New sessions always join this tabbed region, independently of focus/groups.
    default_terminal: Option<RegionId>,
    default_terminal_fraction: f32,
    next_id: RegionId,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Divider {
    pub split: RegionId,
    pub before: usize,
    pub after: usize,
    pub axis: Axis,
    pub parent: UiRect,
    pub position: f32,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub regions: Vec<(RegionId, Content, UiRect)>,
    pub dividers: Vec<Divider>,
}
#[derive(Clone, Debug, PartialEq)]
pub enum Placement {
    Outer(Direction),
    Side(RegionId, Direction),
    Between {
        split: RegionId,
        before: usize,
        after: usize,
    },
}
#[derive(Clone, Debug, PartialEq)]
pub struct DropPlan {
    pub base_version: u64,
    pub layout: ApplicationLayout,
    pub destination: RegionId,
    pub session: u64,
    pub viewport: UiRect,
    pub visibility: Visibility,
    pub sessions: Vec<u64>,
}

impl ApplicationLayout {
    pub fn initial(rect: UiRect, git_width: f32, files_width: f32, terminal_height: f32) -> Self {
        let terminal_height = terminal_height
            .max(120.0)
            .min(rect.height().max(240.0) - 120.0);
        let leaf = |id, content| Node::Leaf { id, content };
        Self {
            root: Node::Split {
                id: 6,
                axis: Axis::Vertical,
                children: vec![
                    Node::Split {
                        id: 5,
                        axis: Axis::Horizontal,
                        children: vec![
                            leaf(2, Content::Git),
                            leaf(1, Content::Editor),
                            leaf(3, Content::Files),
                        ],
                    },
                    leaf(
                        4,
                        Content::Terminal {
                            tabs: vec![],
                            active: None,
                        },
                    ),
                ],
            },
            sizes: HashMap::from([
                (
                    5,
                    vec![
                        git_width / rect.width(),
                        (rect.width() - git_width - files_width).max(200.0) / rect.width(),
                        files_width / rect.width(),
                    ],
                ),
                (
                    6,
                    vec![
                        (rect.height() - terminal_height).max(120.0) / rect.height(),
                        terminal_height / rect.height(),
                    ],
                ),
            ]),
            version: 0,
            default_terminal: Some(4),
            default_terminal_fraction: (terminal_height / rect.height().max(1.0)).clamp(0.1, 0.7),
            next_id: 7,
        }
    }
    fn alloc(&mut self) -> RegionId {
        let id = self.next_id;
        self.next_id += 1;
        id
    }
    pub fn leaves(&self) -> Vec<(RegionId, Content)> {
        let mut out = vec![];
        self.root.leaves(&mut out);
        out
    }
    pub fn terminal_region(&self, session: u64) -> Option<RegionId> {
        self.leaves().into_iter().find_map(|(id, c)| match c {
            Content::Terminal { tabs, .. } if tabs.contains(&session) => Some(id),
            _ => None,
        })
    }
    pub fn terminal_view(&self, id: RegionId) -> Option<(Vec<u64>, Option<u64>)> {
        match self.root.find(id)? {
            Node::Leaf {
                content: Content::Terminal { tabs, active },
                ..
            } => Some((tabs.clone(), *active)),
            _ => None,
        }
    }
    /// New sessions join the default tab strip. Only explicit DropPlans move items
    /// elsewhere; neither the selected terminal nor a legacy group chooses position.
    pub fn attach(&mut self, session: u64) -> RegionId {
        if let Some(existing) = self.terminal_region(session) {
            return existing;
        }
        let region = self
            .default_terminal
            .filter(|id| {
                matches!(
                    self.root.find(*id),
                    Some(Node::Leaf {
                        content: Content::Terminal { .. },
                        ..
                    })
                )
            })
            .unwrap_or_else(|| {
                let region = self.alloc();
                let leaf = Node::Leaf {
                    id: region,
                    content: Content::Terminal {
                        tabs: vec![],
                        active: None,
                    },
                };
                self.wrap(self.root.id(), Direction::Down, leaf);
                self.sizes.insert(
                    self.root.id(),
                    vec![
                        1.0 - self.default_terminal_fraction,
                        self.default_terminal_fraction,
                    ],
                );
                self.default_terminal = Some(region);
                region
            });
        if let Some(Node::Leaf {
            content: Content::Terminal { tabs, active },
            ..
        }) = self.root.find_mut(region)
        {
            tabs.push(session);
            *active = Some(session);
            self.version += 1;
            return region;
        }
        unreachable!("default terminal must be a terminal leaf");
    }
    /// Reconcile creates/closes from existing commands without touching controllers.
    pub fn sync(&mut self, sessions: &[(u64, u64)], active_id: Option<u64>) {
        let before = self.clone();
        let ids = sessions.iter().map(|p| p.0).collect::<HashSet<_>>();
        for (region, content) in self.leaves() {
            if let Content::Terminal { .. } = content
                && let Some(Node::Leaf {
                    content: Content::Terminal { tabs, active },
                    ..
                }) = self.root.find_mut(region)
            {
                tabs.retain(|id| ids.contains(id));
                if !active.is_some_and(|id| tabs.contains(&id)) {
                    *active = tabs.first().copied();
                }
            }
        }
        for &(id, _) in sessions {
            if self.terminal_region(id).is_some() {
                continue;
            }
            self.attach(id);
        }
        if let Some(active_id) = active_id
            && let Some(region) = self.terminal_region(active_id)
            && let Some(Node::Leaf {
                content: Content::Terminal { active, .. },
                ..
            }) = self.root.find_mut(region)
        {
            *active = Some(active_id);
        }
        self.clean();
        if self.root != before.root || self.sizes != before.sizes {
            self.version = before.version + 1;
        }
    }
    pub fn content_rect(&self, snapshot: &Snapshot, kind: &Content) -> Option<UiRect> {
        snapshot
            .regions
            .iter()
            .find(|(_, c, _)| c == kind)
            .map(|(_, _, r)| *r)
    }
    fn wrap(&mut self, target: RegionId, direction: Direction, leaf: Node) -> bool {
        let split = self.alloc();
        let Some(node) = self.root.find_mut(target) else {
            return false;
        };
        let original = node.clone();
        let children = if matches!(direction, Direction::Right | Direction::Down) {
            vec![original, leaf]
        } else {
            vec![leaf, original]
        };
        *node = Node::Split {
            id: split,
            axis: direction.axis(),
            children,
        };
        self.sizes.insert(split, vec![0.5, 0.5]);
        true
    }
    pub fn plan(
        &self,
        session: u64,
        target: &Placement,
        rect: UiRect,
        visible: Visibility,
        sessions: Vec<u64>,
    ) -> Option<DropPlan> {
        let source = self.terminal_region(session)?;
        let only_tab = self.terminal_view(source)?.0.len() == 1;
        let mut layout = self.clone();
        if let Placement::Side(region, _) = *target
            && region == source
            && only_tab
        {
            return None;
        }
        let temporary = layout.alloc();
        let leaf = Node::Leaf {
            id: temporary,
            content: Content::Terminal {
                tabs: vec![session],
                active: Some(session),
            },
        };
        match *target {
            Placement::Outer(dir) => {
                layout.wrap(layout.root.id(), dir, leaf);
            }
            Placement::Side(region, dir) => {
                if !layout.wrap(region, dir, leaf) {
                    return None;
                }
            }
            Placement::Between {
                split,
                before,
                after,
            } => {
                let Some(Node::Split { children, .. }) = layout.root.find_mut(split) else {
                    return None;
                };
                if before >= after || after >= children.len() {
                    return None;
                }
                let sizes = layout.sizes.get_mut(&split)?;
                let extra = (sizes[before] + sizes[after]) / 3.0;
                sizes[before] *= 2.0 / 3.0;
                sizes[after] *= 2.0 / 3.0;
                children.insert(after, leaf);
                sizes.insert(after, extra);
            }
        }
        if let Some(Node::Leaf {
            content: Content::Terminal { tabs, active },
            ..
        }) = layout.root.find_mut(source)
        {
            tabs.retain(|id| *id != session);
            if *active == Some(session) {
                *active = tabs.first().copied();
            }
        }
        layout.clean();
        // A shared source retains its region; an independent terminal retains its ID.
        let destination = if only_tab {
            if let Some(Node::Leaf { id, .. }) = layout.root.find_mut(temporary) {
                *id = source;
            }
            source
        } else {
            temporary
        };
        if !layout.fit(rect, visible) {
            return None;
        }
        layout.version = self.version + 1;
        Some(DropPlan {
            base_version: self.version,
            layout,
            destination,
            session,
            viewport: rect,
            visibility: visible,
            sessions,
        })
    }
    pub fn commit(
        &mut self,
        plan: DropPlan,
        rect: UiRect,
        visibility: Visibility,
        sessions: &[u64],
    ) -> bool {
        if self.version != plan.base_version
            || rect != plan.viewport
            || visibility != plan.visibility
            || sessions != plan.sessions
            || self.terminal_region(plan.session).is_none()
        {
            return false;
        }
        *self = plan.layout;
        true
    }
    fn clean(&mut self) {
        fn clean(node: &mut Node, sizes: &mut HashMap<RegionId, Vec<f32>>) -> bool {
            match node {
                Node::Leaf {
                    content: Content::Terminal { tabs, .. },
                    ..
                } => tabs.is_empty(),
                Node::Leaf { .. } => false,
                Node::Split { id, children, .. } => {
                    let mut removed = vec![];
                    for (i, child) in children.iter_mut().enumerate() {
                        if clean(child, sizes) {
                            removed.push(i);
                        }
                    }
                    let weights = sizes.get_mut(id).unwrap();
                    for i in removed.into_iter().rev() {
                        children.remove(i);
                        let freed = weights.remove(i);
                        if !weights.is_empty() {
                            let near = i.min(weights.len() - 1);
                            weights[near] += freed;
                        }
                    }
                    if children.is_empty() {
                        sizes.remove(id);
                        true
                    } else if children.len() == 1 {
                        let split = *id;
                        *node = children.remove(0);
                        sizes.remove(&split);
                        false
                    } else {
                        children.is_empty()
                    }
                }
            }
        }
        clean(&mut self.root, &mut self.sizes);
        self.default_terminal = self
            .default_terminal
            .filter(|id| self.root.find(*id).is_some());
    }
    pub fn snapshot(&self, rect: UiRect, visible: Visibility) -> Snapshot {
        let mut snapshot = Snapshot {
            regions: vec![],
            dividers: vec![],
        };
        geometry(&self.root, &self.sizes, rect, visible, &mut snapshot);
        snapshot
    }
    fn fit(&mut self, rect: UiRect, visible: Visibility) -> bool {
        let min = minimum(&self.root, visible);
        if min.0 > rect.width() + 0.01 || min.1 > rect.height() + 0.01 {
            return false;
        }
        fn fit(
            node: &Node,
            sizes: &mut HashMap<RegionId, Vec<f32>>,
            rect: UiRect,
            visible: Visibility,
        ) {
            if let Node::Split { id, axis, children } = node {
                let allocations = allocations(children, &sizes[id], *axis, rect, visible);
                let weights = sizes.get_mut(id).unwrap();
                let enabled_weight: f32 = children
                    .iter()
                    .zip(weights.iter())
                    .filter(|(c, _)| enabled(c, visible))
                    .map(|(_, w)| *w)
                    .sum();
                let extent = extent(rect, *axis);
                for (i, child) in children.iter().enumerate() {
                    if enabled(child, visible) {
                        weights[i] = allocations[i] / extent.max(1.0) * enabled_weight;
                    }
                }
                for (child, r) in children.iter().zip(child_rects(rect, *axis, &allocations)) {
                    if enabled(child, visible) {
                        fit(child, sizes, r, visible);
                    }
                }
            }
        }
        fit(&self.root, &mut self.sizes, rect, visible);
        true
    }
    pub fn resize(&mut self, divider: &Divider, position: f32, visible: Visibility) -> bool {
        let Some(Node::Split { axis, children, .. }) = self.root.find(divider.split) else {
            return false;
        };
        let axis = *axis;
        if divider.after >= children.len() {
            return false;
        }
        let allocations = allocations(
            children,
            &self.sizes[&divider.split],
            axis,
            divider.parent,
            visible,
        );
        let total = allocations[divider.before] + allocations[divider.after];
        let first_min = min_extent(&children[divider.before], axis, visible);
        let second_min = min_extent(&children[divider.after], axis, visible);
        if total < first_min + second_min {
            return false;
        }
        let start = match axis {
            Axis::Horizontal => divider.parent.left,
            Axis::Vertical => divider.parent.top,
        };
        let before: f32 = allocations[..divider.before].iter().sum();
        let first = (position - start - before).clamp(first_min, total - second_min);
        if (allocations[divider.before] - first).abs() < 0.01 {
            return false;
        }
        let weights = self.sizes.get_mut(&divider.split).unwrap();
        // The viewport may have shrunk since the last edit. Start from the
        // actual constrained extents, so neighbouring boundaries do not jump.
        let visible_weight: f32 = children
            .iter()
            .zip(weights.iter())
            .filter(|(n, _)| enabled(n, visible))
            .map(|(_, w)| *w)
            .sum();
        for (i, child) in children.iter().enumerate() {
            if enabled(child, visible) {
                weights[i] = allocations[i] / extent(divider.parent, axis) * visible_weight;
            }
        }
        let pair = weights[divider.before] + weights[divider.after];
        let weight = first / total * pair;
        weights[divider.before] = weight;
        weights[divider.after] = pair - weight;
        self.version += 1;
        true
    }
}
fn enabled(node: &Node, v: Visibility) -> bool {
    match node {
        Node::Leaf { content, .. } => match content {
            Content::Git => v.git,
            Content::Files => v.files,
            Content::Terminal { tabs, .. } => v.terminal && !tabs.is_empty(),
            Content::Editor => true,
        },
        Node::Split { children, .. } => children.iter().any(|n| enabled(n, v)),
    }
}
fn minimum(node: &Node, v: Visibility) -> (f32, f32) {
    if !enabled(node, v) {
        return (0.0, 0.0);
    }
    match node {
        Node::Leaf { content, .. } => match content {
            Content::Terminal { .. } => (200.0, 120.0),
            Content::Editor => (200.0, 120.0),
            _ => (100.0, 120.0),
        },
        Node::Split { axis, children, .. } => {
            children
                .iter()
                .map(|n| minimum(n, v))
                .fold((0.0, 0.0), |a, b| match axis {
                    Axis::Horizontal => (a.0 + b.0, a.1.max(b.1)),
                    Axis::Vertical => (a.0.max(b.0), a.1 + b.1),
                })
        }
    }
}
fn min_extent(n: &Node, axis: Axis, v: Visibility) -> f32 {
    let m = minimum(n, v);
    match axis {
        Axis::Horizontal => m.0,
        Axis::Vertical => m.1,
    }
}
fn extent(r: UiRect, a: Axis) -> f32 {
    match a {
        Axis::Horizontal => r.width(),
        Axis::Vertical => r.height(),
    }
}
fn allocations(
    children: &[Node],
    weights: &[f32],
    axis: Axis,
    rect: UiRect,
    v: Visibility,
) -> Vec<f32> {
    let extent = extent(rect, axis).max(0.0);
    let sum: f32 = children
        .iter()
        .zip(weights)
        .filter(|(n, _)| enabled(n, v))
        .map(|(_, w)| *w)
        .sum();
    let mins = children
        .iter()
        .map(|n| min_extent(n, axis, v))
        .collect::<Vec<_>>();
    let min_sum: f32 = mins.iter().sum();
    if min_sum > extent {
        return mins.iter().map(|m| extent * m / min_sum).collect();
    }
    let mut out = children
        .iter()
        .zip(weights)
        .map(|(n, w)| {
            if enabled(n, v) {
                extent * w / sum.max(0.0001)
            } else {
                0.0
            }
        })
        .collect::<Vec<_>>();
    // Water-fill: enforce subtree minima, taking only surplus from other children.
    let deficit: f32 = out.iter().zip(&mins).map(|(a, m)| (m - a).max(0.0)).sum();
    let surplus: f32 = out.iter().zip(&mins).map(|(a, m)| (a - m).max(0.0)).sum();
    for (a, m) in out.iter_mut().zip(mins) {
        *a = if *a < m {
            m
        } else {
            *a - (*a - m) / surplus.max(0.0001) * deficit
        };
    }
    out
}
fn child_rects(rect: UiRect, axis: Axis, allocations: &[f32]) -> Vec<UiRect> {
    let mut position = match axis {
        Axis::Horizontal => rect.left,
        Axis::Vertical => rect.top,
    };
    allocations
        .iter()
        .map(|a| {
            let start = position;
            position += a;
            match axis {
                Axis::Horizontal => UiRect::new(start, rect.top, position, rect.bottom),
                Axis::Vertical => UiRect::new(rect.left, start, rect.right, position),
            }
        })
        .collect()
}
fn geometry(
    node: &Node,
    sizes: &HashMap<RegionId, Vec<f32>>,
    rect: UiRect,
    v: Visibility,
    out: &mut Snapshot,
) {
    if !enabled(node, v) {
        return;
    }
    match node {
        Node::Leaf { id, content } => out.regions.push((*id, content.clone(), rect)),
        Node::Split { id, axis, children } => {
            let allocations = allocations(children, &sizes[id], *axis, rect, v);
            let rects = child_rects(rect, *axis, &allocations);
            let visible = children
                .iter()
                .enumerate()
                .filter(|(_, n)| enabled(n, v))
                .map(|(i, _)| i)
                .collect::<Vec<_>>();
            for pair in visible.windows(2) {
                out.dividers.push(Divider {
                    split: *id,
                    before: pair[0],
                    after: pair[1],
                    axis: *axis,
                    parent: rect,
                    position: match axis {
                        Axis::Horizontal => rects[pair[0]].right,
                        Axis::Vertical => rects[pair[0]].bottom,
                    },
                });
            }
            for (child, rect) in children.iter().zip(rects) {
                geometry(child, sizes, rect, v, out);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const V: Visibility = Visibility {
        git: true,
        files: true,
        terminal: true,
    };
    fn setup() -> (ApplicationLayout, UiRect) {
        let rect = UiRect::new(0.0, 30.0, 1200.0, 800.0);
        let mut layout = ApplicationLayout::initial(rect, 220.0, 220.0, 240.0);
        layout.sync(&[(10, 10), (11, 11)], Some(10));
        (layout, rect)
    }
    fn region(layout: &ApplicationLayout, rect: UiRect, id: u64) -> UiRect {
        layout
            .snapshot(rect, V)
            .regions
            .into_iter()
            .find(|r| r.0 == id)
            .unwrap()
            .2
    }
    #[test]
    fn outer_moves_beyond_drawers_and_preview_is_pure() {
        for dir in [
            Direction::Left,
            Direction::Right,
            Direction::Up,
            Direction::Down,
        ] {
            let (mut layout, rect) = setup();
            let before = layout.clone();
            let plan = layout
                .plan(10, &Placement::Outer(dir), rect, V, vec![10, 11])
                .unwrap();
            assert_eq!(layout, before);
            let r = region(&plan.layout, rect, plan.destination);
            match dir {
                Direction::Left => assert_eq!(r.left, rect.left),
                Direction::Right => assert_eq!(r.right, rect.right),
                Direction::Up => assert_eq!(r.top, rect.top),
                Direction::Down => assert!((r.bottom - rect.bottom).abs() < 0.01),
            }
            let expected = plan.layout.clone();
            assert!(layout.commit(plan, rect, V, &[10, 11]));
            assert_eq!(layout, expected);
        }
    }
    #[test]
    fn narrow_drawer_split_fits_by_redistributing_surplus() {
        let (layout, rect) = setup();
        let plan = layout
            .plan(
                10,
                &Placement::Side(3, Direction::Right),
                rect,
                V,
                vec![10, 11],
            )
            .unwrap();
        assert!(region(&plan.layout, rect, plan.destination).width() >= 199.99);
        assert!(region(&plan.layout, rect, 3).width() >= 99.99);
        assert!(
            layout
                .plan(
                    10,
                    &Placement::Outer(Direction::Right),
                    UiRect::new(0.0, 0.0, 450.0, 300.0),
                    V,
                    vec![10, 11]
                )
                .is_none()
        );
    }
    #[test]
    fn moving_beside_another_terminal_keeps_both_items_and_stable_ids() {
        let (layout, rect) = setup();
        let plan = layout
            .plan(
                10,
                &Placement::Side(1, Direction::Right),
                rect,
                V,
                vec![10, 11],
            )
            .unwrap();
        let destination = plan.destination;
        let layout = plan.layout;
        assert!(
            layout
                .plan(
                    10,
                    &Placement::Side(destination, Direction::Down),
                    rect,
                    V,
                    vec![10, 11]
                )
                .is_none()
        );
        let other = layout.terminal_region(11).unwrap();
        let plan = layout
            .plan(
                10,
                &Placement::Side(other, Direction::Right),
                rect,
                V,
                vec![10, 11],
            )
            .unwrap();
        assert_eq!(plan.layout.terminal_region(10), Some(destination));
        assert_eq!(plan.layout.terminal_region(11), Some(other));
        assert_eq!(
            plan.layout.terminal_view(destination),
            Some((vec![10], Some(10)))
        );
        assert_eq!(plan.layout.terminal_view(other), Some((vec![11], Some(11))));
        let first = region(&plan.layout, rect, other);
        let second = region(&plan.layout, rect, destination);
        assert!((first.right - second.left).abs() < 0.01);
        assert!(
            plan.layout
                .content_rect(&plan.layout.snapshot(rect, V), &Content::Editor)
                .unwrap()
                .width()
                >= 199.99
        );
    }
    #[test]
    fn divider_insertion_resize_and_hidden_regions_keep_topology() {
        let (layout, rect) = setup();
        let divider = layout
            .snapshot(rect, V)
            .dividers
            .into_iter()
            .find(|d| d.split == 5)
            .unwrap();
        let plan = layout
            .plan(
                10,
                &Placement::Between {
                    split: divider.split,
                    before: divider.before,
                    after: divider.after,
                },
                rect,
                V,
                vec![10, 11],
            )
            .unwrap();
        let r = region(&plan.layout, rect, plan.destination);
        assert!(r.left >= region(&plan.layout, rect, 2).right - 0.01);
        assert!(r.right <= region(&plan.layout, rect, 1).left + 0.01);
        let mut layout = plan.layout;
        let topology = layout.root.clone();
        let d = layout
            .snapshot(rect, V)
            .dividers
            .into_iter()
            .find(|d| d.axis == Axis::Horizontal)
            .unwrap();
        layout.resize(&d, d.position + 40.0, V);
        assert_eq!(layout.root, topology);
        let v = Visibility {
            git: false,
            files: false,
            terminal: false,
        };
        let snapshot = layout.snapshot(rect, v);
        assert_eq!(snapshot.regions.len(), 1);
        assert_eq!(snapshot.regions[0].2, rect);
        assert_eq!(layout.root, topology);
    }
    #[test]
    fn stale_model_viewport_visibility_or_closed_session_cannot_commit() {
        let (mut layout, rect) = setup();
        let plan = layout
            .plan(
                10,
                &Placement::Outer(Direction::Left),
                rect,
                V,
                vec![10, 11],
            )
            .unwrap();
        assert!(!layout.commit(plan.clone(), rect, V, &[11]));
        assert!(!layout.commit(
            plan.clone(),
            UiRect::new(0.0, 0.0, 1300.0, 800.0),
            V,
            &[10, 11]
        ));
        assert!(!layout.commit(
            plan.clone(),
            rect,
            Visibility { files: false, ..V },
            &[10, 11]
        ));
        layout.sync(&[(11, 11)], Some(11));
        assert!(!layout.commit(plan, rect, V, &[10, 11]));
    }

    #[test]
    fn resizing_after_window_shrink_keeps_the_other_boundary_fixed() {
        let (mut layout, _) = setup();
        let small = UiRect::new(0.0, 0.0, 500.0, 600.0);
        let before = layout.snapshot(small, V);
        let files = region(&layout, small, 3);
        let sash = before
            .dividers
            .iter()
            .find(|d| d.split == 5 && d.before == 0)
            .unwrap();
        assert!(layout.resize(sash, sash.position + 20.0, V));
        assert!((region(&layout, small, 3).left - files.left).abs() < 0.01);
        assert!((region(&layout, small, 2).right - (sash.position + 20.0)).abs() < 0.01);
    }

    #[test]
    fn new_sessions_share_one_tab_strip_independently_of_legacy_groups() {
        let rect = UiRect::new(0.0, 0.0, 1600.0, 800.0);
        let mut layout = ApplicationLayout::initial(rect, 220.0, 220.0, 240.0);
        layout.sync(&[(1, 1), (2, 1), (3, 3)], Some(1));
        let snapshot = layout.snapshot(rect, V);
        let items = snapshot
            .regions
            .iter()
            .filter(|(_, c, _)| matches!(c, Content::Terminal { .. }))
            .collect::<Vec<_>>();
        assert_eq!(items.len(), 1);
        assert_eq!(layout.terminal_view(4), Some((vec![1, 2, 3], Some(1))));
        layout.sync(&[(1, 1), (2, 1), (3, 3)], Some(3));
        assert_eq!(region(&layout, rect, 4), items[0].2);
        assert_eq!(layout.terminal_view(4), Some((vec![1, 2, 3], Some(3))));
        layout.sync(&[(1, 1), (3, 3)], Some(3));
        assert_eq!(layout.terminal_region(2), None);
        assert_eq!(layout.terminal_region(1), Some(4));
        assert_eq!(layout.terminal_region(3), Some(4));
        assert_eq!(layout.terminal_view(4), Some((vec![1, 3], Some(3))));
    }

    #[test]
    fn new_sessions_stay_in_default_strip_after_selecting_a_moved_terminal() {
        let (mut layout, rect) = setup();
        let plan = layout
            .plan(
                10,
                &Placement::Outer(Direction::Left),
                rect,
                V,
                vec![10, 11],
            )
            .unwrap();
        assert!(layout.commit(plan, rect, V, &[10, 11]));
        let moved = layout.terminal_region(10).unwrap();
        let moved_rect = region(&layout, rect, moved);
        let remaining = layout.terminal_region(11).unwrap();
        // Keyboard creation reconciles while the detached session is active.
        layout.sync(&[(10, 10), (11, 11), (12, 10)], Some(10));
        let new = layout.terminal_region(12).unwrap();
        assert_eq!(region(&layout, rect, moved), moved_rect);
        let dock = region(&layout, rect, remaining);
        let new_rect = region(&layout, rect, new);
        assert_eq!(new, remaining);
        assert_eq!(new_rect, dock);
        assert_eq!(
            layout.terminal_view(remaining),
            Some((vec![11, 12], Some(12)))
        );
    }

    #[test]
    fn new_sessions_recreate_the_default_strip_when_all_previous_items_were_moved() {
        let rect = UiRect::new(0.0, 0.0, 1600.0, 900.0);
        let mut layout = ApplicationLayout::initial(rect, 220.0, 220.0, 240.0);
        layout.sync(&[(10, 10)], Some(10));
        let moved = layout.terminal_region(10).unwrap();
        let plan = layout
            .plan(10, &Placement::Outer(Direction::Left), rect, V, vec![10])
            .unwrap();
        assert!(layout.commit(plan, rect, V, &[10]));
        layout.sync(&[(10, 10), (11, 11)], Some(10));
        let new = layout.terminal_region(11).unwrap();
        let new_rect = region(&layout, rect, new);
        assert!((new_rect.bottom - rect.bottom).abs() < 0.01);
        assert_eq!(new_rect.left, rect.left);
        assert_eq!(new_rect.right, rect.right);
        assert_eq!(layout.terminal_region(10), Some(moved));
        layout.sync(&[(10, 10)], Some(10));
        layout.sync(&[(10, 10), (12, 12), (13, 13)], Some(10));
        let first = region(&layout, rect, layout.terminal_region(12).unwrap());
        let second = region(&layout, rect, layout.terminal_region(13).unwrap());
        assert_eq!(first.top, second.top);
        assert_eq!(first.bottom, second.bottom);
        assert_eq!(first, second);
        assert_eq!(layout.terminal_region(10), Some(moved));
    }

    #[test]
    fn a_single_tab_can_be_detached_beside_its_shared_source() {
        let rect = UiRect::new(0.0, 0.0, 1600.0, 800.0);
        let mut layout = ApplicationLayout::initial(rect, 220.0, 220.0, 240.0);
        layout.sync(&[(1, 1), (2, 1)], Some(1));
        let plan = layout
            .plan(
                1,
                &Placement::Side(4, Direction::Right),
                rect,
                V,
                vec![1, 2],
            )
            .unwrap();
        assert_eq!(layout.terminal_view(4), Some((vec![1, 2], Some(1))));
        assert!(layout.commit(plan, rect, V, &[1, 2]));
        let first = region(&layout, rect, layout.terminal_region(1).unwrap());
        let second = region(&layout, rect, layout.terminal_region(2).unwrap());
        assert!((second.right - first.left).abs() < 0.01);
        assert_eq!(layout.terminal_region(2), Some(4));
        assert_ne!(layout.terminal_region(1), Some(4));
        assert_eq!(layout.terminal_view(4), Some((vec![2], Some(2))));
        layout.attach(3);
        assert_eq!(layout.terminal_view(4), Some((vec![2, 3], Some(3))));
        assert_eq!(
            region(&layout, rect, layout.terminal_region(1).unwrap()),
            first
        );
    }
}
