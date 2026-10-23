//! Renderer-independent syntax, canonical copy text and explicit byte mappings.
use std::ops::Range;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceByte(pub usize);
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DisplayByte(pub usize);
pub type SourceRange = Range<SourceByte>;
pub type DisplayRange = Range<DisplayByte>;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ContentFormat {
    PlainText,
    #[default]
    Markdown,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SyntaxOptions {
    pub tables: bool,
    pub tasks: bool,
    pub strikethrough: bool,
}
impl Default for SyntaxOptions {
    fn default() -> Self {
        Self {
            tables: true,
            tasks: true,
            strikethrough: true,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ParseLimits {
    pub source_bytes: usize,
    pub display_bytes: usize,
    pub retained_bytes: usize,
    pub events: usize,
    pub nodes: usize,
    pub depth: usize,
}
impl ParseLimits {
    pub const fn automatic() -> Self {
        Self {
            source_bytes: 2 * 1024 * 1024,
            display_bytes: 4 * 1024 * 1024,
            retained_bytes: 16 * 1024 * 1024,
            events: 200_000,
            nodes: 200_000,
            depth: 64,
        }
    }
}
impl Default for ParseLimits {
    fn default() -> Self {
        Self::automatic()
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParseStop {
    SourceBudget,
    DisplayBudget,
    RetainedBudget,
    EventBudget,
    NodeBudget,
    DepthBudget,
    Cancelled,
    InvalidStructure,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ParseProgress {
    pub events: usize,
    pub nodes: usize,
    pub display_bytes: usize,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseFailure {
    pub reason: ParseStop,
    pub progress: ParseProgress,
}
impl ParseFailure {
    pub fn message(&self) -> &'static str {
        match self.reason {
            ParseStop::Cancelled => "Markdown processing was superseded.",
            ParseStop::SourceBudget => "This document exceeds the Markdown source limit.",
            ParseStop::DepthBudget => "This document exceeds the Markdown nesting limit.",
            ParseStop::InvalidStructure => "This Markdown structure could not be represented.",
            _ => "This document exceeds the Markdown processing budget.",
        }
    }
}

/// Local to one immutable result; never persist it or reuse it with another result.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NodeId(pub usize);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColumnAlignment {
    None,
    Left,
    Center,
    Right,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NodeKind {
    Document,
    Paragraph,
    Heading {
        level: u8,
    },
    Quote,
    CodeBlock {
        language: Option<String>,
        fenced: bool,
    },
    List {
        start: Option<u64>,
    },
    ListItem {
        ordinal: Option<u64>,
        task: Option<bool>,
    },
    Table {
        columns: Vec<ColumnAlignment>,
    },
    TableRow {
        header: bool,
    },
    TableCell,
    ThematicBreak,
    Text,
    Code,
    Strong,
    Emphasis,
    Strikethrough,
    Link {
        destination: String,
        title: String,
    },
    Image {
        destination: String,
        title: String,
    },
    Html {
        block: bool,
    },
    SoftBreak,
    HardBreak,
    Unsupported,
}
impl NodeKind {
    pub fn is_block(&self) -> bool {
        matches!(
            self,
            Self::Document
                | Self::Paragraph
                | Self::Heading { .. }
                | Self::Quote
                | Self::CodeBlock { .. }
                | Self::List { .. }
                | Self::ListItem { .. }
                | Self::Table { .. }
                | Self::TableRow { .. }
                | Self::TableCell
                | Self::ThematicBreak
                | Self::Html { block: true }
        )
    }
    pub fn attribute_bytes(&self) -> usize {
        match self {
            Self::CodeBlock { language, .. } => language.as_ref().map_or(0, String::capacity),
            Self::Link { destination, title } | Self::Image { destination, title } => {
                destination.capacity() + title.capacity()
            }
            Self::Table { columns } => columns.capacity() * std::mem::size_of::<ColumnAlignment>(),
            _ => 0,
        }
    }
}
/// Flat arena links avoid recursive ownership and recursive destruction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    pub kind: NodeKind,
    pub source: SourceRange,
    pub display: DisplayRange,
    pub parent: Option<NodeId>,
    pub first_child: Option<NodeId>,
    pub next_sibling: Option<NodeId>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MappingKind {
    Exact,
    Transformed,
    Synthetic,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MappingSegment {
    pub display: DisplayRange,
    pub source: SourceRange,
    pub kind: MappingKind,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Heading {
    pub node: NodeId,
    pub level: u8,
    pub source: SourceRange,
    pub display: DisplayRange,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiagnosticKind {
    LiteralHtml,
    UnsupportedSyntax,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub kind: DiagnosticKind,
    pub source: SourceRange,
}

#[derive(Clone, Debug)]
pub struct MarkdownDocument {
    pub source: Arc<String>,
    pub format: ContentFormat,
    pub nodes: Vec<Node>,
    pub display_text: String,
    pub mapping: Vec<MappingSegment>,
    pub headings: Vec<Heading>,
    pub diagnostics: Vec<Diagnostic>,
    /// Mapping indices sorted by source start for reverse lookup.
    pub(crate) source_order: Vec<usize>,
    pub progress: ParseProgress,
}
impl MarkdownDocument {
    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id.0)
    }
    pub fn text(&self, range: DisplayRange) -> Option<&str> {
        self.display_text.get(range.start.0..range.end.0)
    }
    pub fn children(&self, id: NodeId) -> Children<'_> {
        Children {
            document: self,
            next: self.node(id).and_then(|n| n.first_child),
        }
    }
    pub fn retained_bytes(&self) -> usize {
        self.source.capacity()
            + self.display_text.capacity()
            + self.nodes.capacity() * std::mem::size_of::<Node>()
            + self
                .nodes
                .iter()
                .map(|n| n.kind.attribute_bytes())
                .sum::<usize>()
            + self.mapping.capacity() * std::mem::size_of::<MappingSegment>()
            + self.source_order.capacity() * std::mem::size_of::<usize>()
            + self.headings.capacity() * std::mem::size_of::<Heading>()
            + self.diagnostics.capacity() * std::mem::size_of::<Diagnostic>()
    }
    pub fn source_at(&self, offset: DisplayByte) -> Option<SourceRange> {
        if !self.display_text.is_char_boundary(offset.0) {
            return None;
        }
        if offset.0 == self.display_text.len() {
            return Some(SourceByte(self.source.len())..SourceByte(self.source.len()));
        }
        let index = self.mapping.partition_point(|s| s.display.end <= offset);
        let segment = self
            .mapping
            .get(index)
            .filter(|s| s.display.start <= offset && offset < s.display.end)?;
        Some(match segment.kind {
            MappingKind::Exact => {
                let start = segment.source.start.0 + offset.0 - segment.display.start.0;
                let length = self.display_text[offset.0..].chars().next()?.len_utf8();
                SourceByte(start)..SourceByte(start + length)
            }
            MappingKind::Transformed | MappingKind::Synthetic => segment.source.clone(),
        })
    }
    pub fn display_at(&self, offset: SourceByte, after: bool) -> Option<DisplayByte> {
        if offset.0 > self.source.len() || !self.source.is_char_boundary(offset.0) {
            return None;
        }
        let end = self
            .source_order
            .partition_point(|i| self.mapping[*i].source.start <= offset);
        let candidates = &self.source_order[..end];
        // Prefer an exact containing event; delimiters and synthetic markers use
        // the nearest preceding source anchor, never a guessed exact position.
        for &index in candidates.iter().rev() {
            let s = &self.mapping[index];
            if s.source.start <= offset && offset < s.source.end {
                return Some(if s.kind == MappingKind::Exact {
                    DisplayByte(s.display.start.0 + offset.0 - s.source.start.0)
                } else if after {
                    s.display.end
                } else {
                    s.display.start
                });
            }
            if s.source.end <= offset {
                return Some(s.display.end);
            }
        }
        Some(DisplayByte(0))
    }
}
pub struct Children<'a> {
    document: &'a MarkdownDocument,
    next: Option<NodeId>,
}
impl Iterator for Children<'_> {
    type Item = NodeId;
    fn next(&mut self) -> Option<Self::Item> {
        let id = self.next?;
        self.next = self.document.node(id)?.next_sibling;
        Some(id)
    }
}
