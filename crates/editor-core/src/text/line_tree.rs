//! Persistent blocks of line metadata. Byte offsets are subtree sums, so an
//! edit never shifts every later line or copies an unchanged snapshot's blocks.
use super::lines::Position;
use std::{ops::Range, sync::Arc};

const BLOCK: usize = 512;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Line {
    pub bytes: usize,
    pub width: u32,
    pub crlf: bool,
    pub positions: Option<Arc<Vec<Position>>>,
}
impl Line {
    pub fn positions(&self) -> &[Position] {
        self.positions.as_deref().map_or(&[], Vec::as_slice)
    }
}

#[derive(Clone, Debug)]
enum Kind {
    Leaf {
        lines: Vec<Line>,
        starts: Vec<usize>,
    },
    Branch(Arc<Node>, Arc<Node>),
}
#[derive(Clone, Debug)]
struct Node {
    rows: usize,
    bytes: usize,
    width: u32,
    crlf: usize,
    height: usize,
    kind: Kind,
}
type Link = Option<Arc<Node>>;

fn leaf(lines: Vec<Line>) -> Arc<Node> {
    let mut bytes = 0;
    let starts = lines
        .iter()
        .map(|line| {
            let at = bytes;
            bytes += line.bytes;
            at
        })
        .collect();
    Arc::new(Node {
        rows: lines.len(),
        bytes,
        width: lines.iter().map(|l| l.width).max().unwrap_or(0),
        crlf: lines.iter().filter(|l| l.crlf).count(),
        height: 1,
        kind: Kind::Leaf { lines, starts },
    })
}
fn branch(left: Arc<Node>, right: Arc<Node>) -> Arc<Node> {
    Arc::new(Node {
        rows: left.rows + right.rows,
        bytes: left.bytes + right.bytes,
        width: left.width.max(right.width),
        crlf: left.crlf + right.crlf,
        height: 1 + left.height.max(right.height),
        kind: Kind::Branch(left, right),
    })
}
fn balanced(left: Arc<Node>, right: Arc<Node>) -> Arc<Node> {
    if left.height > right.height + 1 {
        let Kind::Branch(a, b) = &left.kind else {
            unreachable!()
        };
        if a.height >= b.height {
            return branch(a.clone(), branch(b.clone(), right));
        }
        let Kind::Branch(b1, b2) = &b.kind else {
            unreachable!()
        };
        return branch(branch(a.clone(), b1.clone()), branch(b2.clone(), right));
    }
    if right.height > left.height + 1 {
        let Kind::Branch(a, b) = &right.kind else {
            unreachable!()
        };
        if b.height >= a.height {
            return branch(branch(left, a.clone()), b.clone());
        }
        let Kind::Branch(a1, a2) = &a.kind else {
            unreachable!()
        };
        return branch(branch(left, a1.clone()), branch(a2.clone(), b.clone()));
    }
    branch(left, right)
}
fn join(left: Link, right: Link) -> Link {
    let (Some(left), Some(right)) = (&left, &right) else {
        return left.or(right);
    };
    if left.height > right.height + 1 {
        let Kind::Branch(a, b) = &left.kind else {
            unreachable!()
        };
        return Some(balanced(
            a.clone(),
            join(Some(b.clone()), Some(right.clone())).unwrap(),
        ));
    }
    if right.height > left.height + 1 {
        let Kind::Branch(a, b) = &right.kind else {
            unreachable!()
        };
        return Some(balanced(
            join(Some(left.clone()), Some(a.clone())).unwrap(),
            b.clone(),
        ));
    }
    Some(branch(left.clone(), right.clone()))
}
fn split(root: Link, row: usize) -> (Link, Link) {
    let Some(node) = root else {
        return (None, None);
    };
    if row == 0 {
        return (None, Some(node));
    }
    if row == node.rows {
        return (Some(node), None);
    }
    match &node.kind {
        Kind::Leaf { lines, .. } => (
            Some(leaf(lines[..row].to_vec())),
            Some(leaf(lines[row..].to_vec())),
        ),
        Kind::Branch(left, right) => {
            if row < left.rows {
                let (a, b) = split(Some(left.clone()), row);
                (a, join(b, Some(right.clone())))
            } else {
                let (a, b) = split(Some(right.clone()), row - left.rows);
                (join(Some(left.clone()), a), b)
            }
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(super) struct LineTree {
    root: Link,
}
impl PartialEq for LineTree {
    fn eq(&self, other: &Self) -> bool {
        self.iter().eq(other.iter())
    }
}
impl Eq for LineTree {}
impl LineTree {
    pub fn from_lines(lines: impl IntoIterator<Item = Line>) -> Self {
        let lines: Vec<_> = lines.into_iter().collect();
        let count = lines.len().div_ceil(BLOCK);
        let mut remaining = lines.len();
        let mut lines = lines.into_iter();
        let mut leaves = Vec::new();
        // Distribute rows evenly rather than leaving a tiny last block at an
        // internal splice boundary. Every multi-block rebuild is at least half full.
        for i in 0..count {
            let size = remaining.div_ceil(count - i);
            let block: Vec<_> = lines.by_ref().take(size).collect();
            remaining -= size;
            leaves.push(leaf(block));
        }
        fn build(nodes: &[Arc<Node>]) -> Link {
            match nodes.len() {
                0 => None,
                1 => Some(nodes[0].clone()),
                n => Some(branch(
                    build(&nodes[..n / 2]).unwrap(),
                    build(&nodes[n / 2..]).unwrap(),
                )),
            }
        }
        Self {
            root: build(&leaves),
        }
    }
    pub fn len(&self) -> usize {
        self.root.as_ref().map_or(0, |n| n.rows)
    }
    pub fn max_width(&self) -> usize {
        self.root.as_ref().map_or(0, |n| n.width as usize)
    }
    pub fn crlf_count(&self) -> usize {
        self.root.as_ref().map_or(0, |n| n.crlf)
    }
    fn block(&self, mut row: usize) -> (usize, usize, &[Line], &[usize]) {
        let (mut base, mut bytes) = (0, 0);
        let mut node = self.root.as_deref().unwrap();
        loop {
            match &node.kind {
                Kind::Leaf { lines, starts } => return (base, bytes, lines, starts),
                Kind::Branch(left, right) => {
                    if row < left.rows {
                        node = left;
                    } else {
                        row -= left.rows;
                        base += left.rows;
                        bytes += left.bytes;
                        node = right;
                    }
                }
            }
        }
    }
    pub fn get(&self, row: usize) -> &Line {
        let (base, _, lines, _) = self.block(row);
        &lines[row - base]
    }
    pub fn start(&self, row: usize) -> usize {
        let (base, bytes, _, starts) = self.block(row);
        bytes + starts[row - base]
    }
    pub fn line_of(&self, mut offset: usize) -> usize {
        let mut base = 0;
        let mut node = self.root.as_deref().unwrap();
        loop {
            match &node.kind {
                Kind::Leaf { starts, .. } => {
                    return base + starts.partition_point(|&s| s <= offset).saturating_sub(1);
                }
                Kind::Branch(left, right) => {
                    if offset < left.bytes {
                        node = left;
                    } else {
                        offset -= left.bytes;
                        base += left.rows;
                        node = right;
                    }
                }
            }
        }
    }
    pub fn extend(&mut self, lines: Vec<Line>) {
        self.root = join(self.root.take(), Self::from_lines(lines).root);
    }
    pub fn replace(&mut self, range: Range<usize>, replacement: Self) {
        if self.len() == 0 {
            assert_eq!(range, 0..0);
            *self = replacement;
            return;
        }
        if range.len() == 1 && replacement.len() == 1 {
            let replacement = replacement.get(0).clone();
            self.update_line(range.start, |line| *line = replacement);
            return;
        }
        // Include neighbouring blocks so repeated insertions do not leave a new
        // tiny leaf behind on every edit. More distant blocks stay shared.
        let (mut first, _, _, _) = self.block(range.start.min(self.len() - 1));
        let (last, _, right, _) = self.block(
            range
                .end
                .saturating_sub(1)
                .max(range.start)
                .min(self.len() - 1),
        );
        let mut end = last + right.len();
        if first > 0 {
            first = self.block(first - 1).0;
        }
        if end < self.len() {
            let (base, _, lines, _) = self.block(end);
            end = base + lines.len();
        }
        let rows = (first..range.start)
            .map(|row| self.get(row).clone())
            .chain(replacement.iter().cloned())
            .chain((range.end..end).map(|row| self.get(row).clone()));
        let rebuilt = Self::from_lines(rows);
        let (before, rest) = split(self.root.take(), first);
        let (_, after) = split(rest, end - first);
        self.root = join(join(before, rebuilt.root), after);
    }
    pub fn update_line(&mut self, row: usize, update: impl FnOnce(&mut Line)) {
        fn visit(node: &mut Arc<Node>, row: usize, update: impl FnOnce(&mut Line)) {
            let node = Arc::make_mut(node);
            match &mut node.kind {
                Kind::Leaf { lines, starts } => {
                    update(&mut lines[row]);
                    let mut bytes = 0;
                    for (line, start) in lines.iter().zip(starts) {
                        *start = bytes;
                        bytes += line.bytes;
                    }
                    node.bytes = bytes;
                    node.width = lines.iter().map(|l| l.width).max().unwrap_or(0);
                    node.crlf = lines.iter().filter(|l| l.crlf).count();
                }
                Kind::Branch(left, right) => {
                    if row < left.rows {
                        visit(left, row, update);
                    } else {
                        visit(right, row - left.rows, update);
                    }
                    node.bytes = left.bytes + right.bytes;
                    node.width = left.width.max(right.width);
                    node.crlf = left.crlf + right.crlf;
                }
            }
        }
        visit(self.root.as_mut().unwrap(), row, update);
    }
    pub fn iter(&self) -> Iter<'_> {
        Iter {
            stack: self.root.as_deref().into_iter().collect(),
            current: [].iter(),
        }
    }
}
pub(super) struct Iter<'a> {
    stack: Vec<&'a Node>,
    current: std::slice::Iter<'a, Line>,
}
impl<'a> Iterator for Iter<'a> {
    type Item = &'a Line;
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(line) = self.current.next() {
                return Some(line);
            }
            match &self.stack.pop()?.kind {
                Kind::Leaf { lines, .. } => self.current = lines.iter(),
                Kind::Branch(left, right) => {
                    self.stack.push(right);
                    self.stack.push(left);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn row(i: usize) -> Line {
        Line {
            bytes: i % 13 + 1,
            width: (i % 97) as u32,
            crlf: i.is_multiple_of(3),
            positions: None,
        }
    }
    #[test]
    fn a_local_edit_shares_distant_blocks_with_older_snapshots() {
        let mut tree = LineTree::from_lines((0..10000).map(row));
        let old = tree.clone();
        tree.replace(0..1, LineTree::from_lines([row(77)]));
        assert_eq!(old.get(0), &row(0));
        assert_eq!(tree.get(0), &row(77));
        assert!(std::ptr::eq(old.get(9999), tree.get(9999)));
        verify(tree.root.as_ref().unwrap());
    }

    #[test]
    fn repeated_front_insertions_keep_blocks_compact_and_balanced() {
        let mut tree = LineTree::from_lines((0..1000).map(row));
        for i in 0..2000 {
            tree.replace(0..0, LineTree::from_lines([row(i)]));
        }
        fn leaves(node: &Node) -> usize {
            match &node.kind {
                Kind::Leaf { .. } => 1,
                Kind::Branch(a, b) => leaves(a) + leaves(b),
            }
        }
        assert_eq!(tree.len(), 3000);
        assert!(leaves(tree.root.as_ref().unwrap()) <= 2 * tree.len().div_ceil(BLOCK));
        assert_eq!(tree.get(0), &row(1999));
        assert_eq!(tree.get(2999), &row(999));
        verify(tree.root.as_ref().unwrap());
    }
    fn verify(node: &Node) {
        match &node.kind {
            Kind::Leaf { lines, starts } => {
                assert!(!lines.is_empty() && lines.len() <= BLOCK);
                assert_eq!(starts.len(), lines.len());
                assert_eq!(node.height, 1);
            }
            Kind::Branch(a, b) => {
                verify(a);
                verify(b);
                assert!(a.height.abs_diff(b.height) <= 1);
                assert_eq!(node.height, 1 + a.height.max(b.height));
                assert_eq!(node.rows, a.rows + b.rows);
                assert_eq!(node.bytes, a.bytes + b.bytes);
                assert_eq!(node.width, a.width.max(b.width));
                assert_eq!(node.crlf, a.crlf + b.crlf);
            }
        }
    }
    #[test]
    fn splices_preserve_snapshots_and_aggregate_queries() {
        let mut flat: Vec<_> = (0..10000).map(row).collect();
        let original = LineTree::from_lines(flat.clone());
        let mut tree = original.clone();
        let mut seed = 7u64;
        for i in 0..200 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let start = seed as usize % flat.len();
            let end = (start + (seed >> 32) as usize % 70).min(flat.len());
            let inserted: Vec<_> = (0..i % 47).map(|n| row(n + i)).collect();
            tree.replace(start..end, LineTree::from_lines(inserted.clone()));
            flat.splice(start..end, inserted);
            assert!(tree.iter().eq(flat.iter()));
            assert_eq!(
                tree.max_width(),
                flat.iter().map(|l| l.width as usize).max().unwrap_or(0)
            );
            assert_eq!(tree.crlf_count(), flat.iter().filter(|l| l.crlf).count());
            let mut offset = 0;
            for (r, line) in flat.iter().enumerate() {
                assert_eq!(tree.start(r), offset);
                assert_eq!(tree.line_of(offset), r);
                assert_eq!(tree.get(r), line);
                offset += line.bytes;
            }
            verify(tree.root.as_ref().unwrap());
        }
        assert!(
            original
                .iter()
                .eq((0..10000).map(row).collect::<Vec<_>>().iter())
        );
    }
}
