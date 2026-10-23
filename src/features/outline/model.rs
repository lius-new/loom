//! The structure of one document as a tree. Ranges are byte offsets into the
//! text the tree was built from; a caller drops the tree once that text changes.
use std::collections::HashSet;
use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    File,
    Module,
    Namespace,
    Package,
    Class,
    Method,
    Property,
    Field,
    Constructor,
    Enum,
    Interface,
    Function,
    Variable,
    Constant,
    String,
    Number,
    Boolean,
    Array,
    Object,
    Key,
    Null,
    EnumMember,
    Struct,
    Event,
    Operator,
    TypeParameter,
    /// A Markdown heading of the given level, 1 to 6.
    Heading(u8),
}

impl Kind {
    /// The kind for the protocol's `SymbolKind` number.
    pub fn from_lsp(kind: u32) -> Self {
        match kind {
            1 => Self::File,
            2 => Self::Module,
            3 => Self::Namespace,
            4 => Self::Package,
            5 => Self::Class,
            6 => Self::Method,
            7 => Self::Property,
            8 => Self::Field,
            9 => Self::Constructor,
            10 => Self::Enum,
            11 => Self::Interface,
            12 => Self::Function,
            13 => Self::Variable,
            14 => Self::Constant,
            15 => Self::String,
            16 => Self::Number,
            17 => Self::Boolean,
            18 => Self::Array,
            19 => Self::Object,
            20 => Self::Key,
            21 => Self::Null,
            22 => Self::EnumMember,
            23 => Self::Struct,
            24 => Self::Event,
            25 => Self::Operator,
            26 => Self::TypeParameter,
            _ => Self::Variable,
        }
    }
}

/// A symbol with its children, as a producer hands it over.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub name: String,
    pub detail: Option<String>,
    pub kind: Kind,
    pub deprecated: bool,
    /// Everything the symbol covers, children included.
    pub full: Range<usize>,
    /// What navigation lands on: usually the name.
    pub select: Range<usize>,
    pub children: Vec<Item>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    pub name: String,
    pub detail: Option<String>,
    pub kind: Kind,
    pub deprecated: bool,
    pub full: Range<usize>,
    pub select: Range<usize>,
    pub depth: u16,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    /// Stable across rebuilds of the same document, so a collapsed branch stays
    /// collapsed while the person types.
    pub key: String,
    local: String,
}

/// A visible line of the tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row {
    pub node: usize,
    pub depth: u16,
    pub has_children: bool,
    pub expanded: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Outline {
    nodes: Vec<Node>,
    roots: Vec<usize>,
}

impl Outline {
    pub fn new(items: Vec<Item>) -> Self {
        let mut outline = Self::default();
        for item in items {
            outline.add(item, None, 0);
        }
        outline
    }

    fn add(&mut self, item: Item, parent: Option<usize>, depth: u16) -> usize {
        let local = format!("{:?}:{}", item.kind, item.name);
        let siblings = match parent {
            Some(parent) => &self.nodes[parent].children,
            None => &self.roots,
        };
        let twins = siblings
            .iter()
            .filter(|&&sibling| self.nodes[sibling].local == local)
            .count();
        let mut key = parent.map_or_else(String::new, |parent| self.nodes[parent].key.clone());
        key.push('/');
        key.push_str(&local);
        if twins > 0 {
            key.push_str(&format!("#{twins}"));
        }
        let index = self.nodes.len();
        self.nodes.push(Node {
            name: item.name,
            detail: item.detail,
            kind: item.kind,
            deprecated: item.deprecated,
            full: item.full,
            select: item.select,
            depth,
            parent,
            children: Vec::new(),
            key,
            local,
        });
        match parent {
            Some(parent) => self.nodes[parent].children.push(index),
            None => self.roots.push(index),
        }
        for child in item.children {
            self.add(child, Some(index), depth + 1);
        }
        index
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn node(&self, index: usize) -> Option<&Node> {
        self.nodes.get(index)
    }

    /// Keys of every node that has children: what "collapse all" stores.
    pub fn branch_keys(&self) -> HashSet<String> {
        self.nodes
            .iter()
            .filter(|node| !node.children.is_empty())
            .map(|node| node.key.clone())
            .collect()
    }

    /// The lines to show, in order. `sorted` orders each level by name.
    pub fn rows(&self, collapsed: &HashSet<String>, sorted: bool) -> Vec<Row> {
        let mut rows = Vec::with_capacity(self.nodes.len());
        let mut stack = self.ordered(&self.roots, sorted);
        stack.reverse();
        while let Some(index) = stack.pop() {
            let node = &self.nodes[index];
            let has_children = !node.children.is_empty();
            let expanded = has_children && !collapsed.contains(&node.key);
            rows.push(Row {
                node: index,
                depth: node.depth,
                has_children,
                expanded,
            });
            if expanded {
                let mut children = self.ordered(&node.children, sorted);
                children.reverse();
                stack.extend(children);
            }
        }
        rows
    }

    fn ordered(&self, level: &[usize], sorted: bool) -> Vec<usize> {
        let mut level = level.to_vec();
        if sorted {
            level.sort_by_cached_key(|&index| self.nodes[index].name.to_lowercase());
        }
        level
    }

    /// The deepest node covering `offset`.
    pub fn containing(&self, offset: usize) -> Option<usize> {
        let mut found = None;
        let mut level = &self.roots;
        while let Some(&hit) = level.iter().find(|&&index| {
            let full = &self.nodes[index].full;
            full.start <= offset && offset <= full.end
        }) {
            found = Some(hit);
            level = &self.nodes[hit].children;
        }
        found
    }

    /// The node to mark for `node` when some ancestor is collapsed: the
    /// outermost collapsed ancestor stands in for what is hidden inside it.
    pub fn visible_stand_in(&self, node: usize, collapsed: &HashSet<String>) -> usize {
        let mut chain = Vec::new();
        let mut at = Some(node);
        while let Some(index) = at {
            chain.push(index);
            at = self.nodes[index].parent;
        }
        chain
            .iter()
            .rev()
            .copied()
            .find(|&index| {
                index != node
                    && !self.nodes[index].children.is_empty()
                    && collapsed.contains(&self.nodes[index].key)
            })
            .unwrap_or(node)
    }
}

/// Builds items from flat symbols (a server's `SymbolInformation`) by nesting
/// each one in the smallest earlier symbol that covers it.
pub fn nest(mut flat: Vec<Item>) -> Vec<Item> {
    flat.sort_by(|a, b| {
        a.full
            .start
            .cmp(&b.full.start)
            .then(b.full.end.cmp(&a.full.end))
    });
    let mut roots: Vec<Item> = Vec::new();
    // Indices into the tree being built, from the outermost open symbol down.
    let mut open: Vec<Vec<usize>> = Vec::new();
    let mut ends: Vec<usize> = Vec::new();
    for item in flat {
        while ends.last().is_some_and(|&end| item.full.start >= end) {
            ends.pop();
            open.pop();
        }
        let end = item.full.end;
        let path = match open.last() {
            None => {
                roots.push(item);
                vec![roots.len() - 1]
            }
            Some(parent) => {
                let mut path = parent.clone();
                let children = &mut children_at(&mut roots, parent).children;
                children.push(item);
                path.push(children.len() - 1);
                path
            }
        };
        open.push(path);
        ends.push(end);
    }
    roots
}

fn children_at<'a>(roots: &'a mut [Item], path: &[usize]) -> &'a mut Item {
    let mut item = &mut roots[path[0]];
    for &step in &path[1..] {
        item = &mut item.children[step];
    }
    item
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(name: &str, full: Range<usize>, children: Vec<Item>) -> Item {
        Item {
            name: name.to_owned(),
            detail: None,
            kind: Kind::Function,
            deprecated: false,
            select: full.start..full.start,
            full,
            children,
        }
    }

    fn sample() -> Outline {
        Outline::new(vec![
            item(
                "Zeta",
                0..50,
                vec![item("b", 5..20, vec![]), item("a", 21..40, vec![])],
            ),
            item("Alpha", 51..80, vec![]),
        ])
    }

    #[test]
    fn rows_follow_document_order_and_hide_collapsed_branches() {
        let outline = sample();
        let names = |rows: Vec<Row>| {
            rows.iter()
                .map(|row| outline.node(row.node).unwrap().name.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            names(outline.rows(&HashSet::new(), false)),
            ["Zeta", "b", "a", "Alpha"]
        );
        assert_eq!(
            names(outline.rows(&outline.branch_keys(), false)),
            ["Zeta", "Alpha"]
        );
    }

    #[test]
    fn sorting_orders_each_level_by_name_ignoring_case() {
        let outline = sample();
        let rows = outline.rows(&HashSet::new(), true);
        let names = rows
            .iter()
            .map(|row| outline.node(row.node).unwrap().name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(names, ["Alpha", "Zeta", "a", "b"]);
    }

    #[test]
    fn containing_finds_the_deepest_node_and_stand_in_skips_collapsed_ones() {
        let outline = sample();
        let inner = outline.containing(25).unwrap();
        assert_eq!(outline.node(inner).unwrap().name, "a");
        assert_eq!(outline.containing(500), None);
        let collapsed = outline.branch_keys();
        let shown = outline.visible_stand_in(inner, &collapsed);
        assert_eq!(outline.node(shown).unwrap().name, "Zeta");
    }

    #[test]
    fn duplicate_names_keep_distinct_stable_keys() {
        let outline = Outline::new(vec![item("f", 0..5, vec![]), item("f", 6..10, vec![])]);
        let keys = outline.rows(&HashSet::new(), false);
        let first = &outline.node(keys[0].node).unwrap().key;
        let second = &outline.node(keys[1].node).unwrap().key;
        assert_ne!(first, second);
        assert_eq!(
            Outline::new(vec![
                item("f", 100..105, vec![]),
                item("f", 106..110, vec![])
            ])
            .node(0)
            .unwrap()
            .key,
            *first
        );
    }

    #[test]
    fn flat_symbols_nest_by_the_range_that_covers_them() {
        let flat = vec![
            item("method", 10..20, vec![]),
            item("Type", 0..40, vec![]),
            item("other", 45..60, vec![]),
            item("field", 22..30, vec![]),
        ];
        let nested = nest(flat);
        assert_eq!(nested.len(), 2);
        assert_eq!(nested[0].name, "Type");
        let kids = nested[0]
            .children
            .iter()
            .map(|child| child.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(kids, ["method", "field"]);
        assert_eq!(nested[1].name, "other");
    }

    #[test]
    fn lsp_kinds_map_and_unknown_numbers_fall_back_to_variable() {
        assert_eq!(Kind::from_lsp(5), Kind::Class);
        assert_eq!(Kind::from_lsp(12), Kind::Function);
        assert_eq!(Kind::from_lsp(99), Kind::Variable);
    }
}
