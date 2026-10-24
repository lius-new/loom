//! The only adapter allowed to depend on pulldown-cmark's event types.
use crate::features::markdown::html;
use crate::features::markdown::model::*;
use pulldown_cmark::{Alignment, CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use std::ops::Range;
use std::sync::Arc;

type HtmlBlock = (Range<usize>, Vec<(String, Range<usize>)>);

struct Frame {
    node: NodeId,
    last_child: Option<NodeId>,
    next_ordinal: Option<u64>,
    cells: usize,
}
struct Builder {
    document: MarkdownDocument,
    stack: Vec<Frame>,
    limits: ParseLimits,
    attribute_bytes: usize,
}
impl Builder {
    fn new(source: Arc<String>, format: ContentFormat, limits: ParseLimits) -> Self {
        let end = source.len();
        Self {
            document: MarkdownDocument {
                source,
                format,
                nodes: vec![Node {
                    kind: NodeKind::Document,
                    source: SourceByte(0)..SourceByte(end),
                    display: DisplayByte(0)..DisplayByte(0),
                    parent: None,
                    first_child: None,
                    next_sibling: None,
                }],
                display_text: String::new(),
                mapping: Vec::new(),
                headings: Vec::new(),
                diagnostics: Vec::new(),
                alignments: Vec::new(),
                source_order: Vec::new(),
                progress: ParseProgress::default(),
            },
            stack: vec![Frame {
                node: NodeId(0),
                last_child: None,
                next_ordinal: None,
                cells: 0,
            }],
            limits,
            attribute_bytes: 0,
        }
    }
    fn failure(&self, reason: ParseStop) -> ParseFailure {
        ParseFailure {
            reason,
            progress: self.document.progress,
        }
    }
    fn node(&mut self, kind: NodeKind, source: Range<usize>) -> Result<NodeId, ParseFailure> {
        if self.document.nodes.len() >= self.limits.nodes {
            return Err(self.failure(ParseStop::NodeBudget));
        }
        self.attribute_bytes = self.attribute_bytes.saturating_add(kind.attribute_bytes());
        let parent = self.stack.last_mut().expect("document frame");
        let id = NodeId(self.document.nodes.len());
        let at = DisplayByte(self.document.display_text.len());
        if let Some(previous) = parent.last_child {
            self.document.nodes[previous.0].next_sibling = Some(id);
        } else {
            self.document.nodes[parent.node.0].first_child = Some(id);
        }
        parent.last_child = Some(id);
        self.document.nodes.push(Node {
            kind,
            source: SourceByte(source.start)..SourceByte(source.end),
            display: at..at,
            parent: Some(parent.node),
            first_child: None,
            next_sibling: None,
        });
        self.document.progress.nodes = self.document.nodes.len();
        Ok(id)
    }
    fn start(&mut self, kind: NodeKind, source: Range<usize>) -> Result<(), ParseFailure> {
        if self.stack.len() > self.limits.depth {
            return Err(self.failure(ParseStop::DepthBudget));
        }
        let next_ordinal = match &kind {
            NodeKind::List { start } => *start,
            _ => None,
        };
        let id = self.node(kind, source)?;
        self.stack.push(Frame {
            node: id,
            last_child: None,
            next_ordinal,
            cells: 0,
        });
        Ok(())
    }
    fn push(
        &mut self,
        text: &str,
        source: Range<usize>,
        synthetic: bool,
    ) -> Result<(), ParseFailure> {
        if self.document.display_text.len().saturating_add(text.len()) > self.limits.display_bytes {
            return Err(self.failure(ParseStop::DisplayBudget));
        }
        if text.is_empty() {
            return Ok(());
        }
        let start = self.document.display_text.len();
        self.document.display_text.push_str(text);
        let kind = if synthetic {
            MappingKind::Synthetic
        } else if self.document.source.get(source.clone()) == Some(text) {
            MappingKind::Exact
        } else {
            MappingKind::Transformed
        };
        self.document.mapping.push(MappingSegment {
            display: DisplayByte(start)..DisplayByte(self.document.display_text.len()),
            source: SourceByte(source.start)..SourceByte(source.end),
            kind,
        });
        self.document.progress.display_bytes = self.document.display_text.len();
        Ok(())
    }
    fn leaf(
        &mut self,
        kind: NodeKind,
        text: &str,
        source: Range<usize>,
        synthetic: bool,
    ) -> Result<(), ParseFailure> {
        let id = self.node(kind, source.clone())?;
        self.push(text, source, synthetic)?;
        self.document.nodes[id.0].display.end = DisplayByte(self.document.display_text.len());
        Ok(())
    }
    fn newline(&mut self, anchor: usize) -> Result<(), ParseFailure> {
        if !self.document.display_text.is_empty() && !self.document.display_text.ends_with('\n') {
            self.push("\n", anchor..anchor, true)?;
        }
        Ok(())
    }
    fn finish_node(&mut self) -> Result<NodeId, ParseFailure> {
        if self.stack.len() <= 1 {
            return Err(self.failure(ParseStop::InvalidStructure));
        }
        let frame = self.stack.pop().unwrap();
        let node = &mut self.document.nodes[frame.node.0];
        node.display.end = DisplayByte(self.document.display_text.len());
        if let NodeKind::Heading { level } = node.kind {
            self.document.headings.push(Heading {
                node: frame.node,
                level,
                source: node.source.clone(),
                display: node.display.clone(),
            });
        }
        Ok(frame.node)
    }
    /// Records that the innermost open node is centered or right-aligned.
    fn mark_alignment(&mut self, align: ColumnAlignment) {
        if matches!(align, ColumnAlignment::Left | ColumnAlignment::None) {
            return;
        }
        if let Some(frame) = self.stack.last() {
            self.document.alignments.push((frame.node, align));
        }
    }
    /// Builds the nodes of an interpreted HTML block. `parts` maps offsets in
    /// the planned text to source offsets as (planned start, source start, length).
    fn apply_html(
        &mut self,
        ops: Vec<html::Op>,
        parts: &[(usize, usize, usize)],
    ) -> Result<(), ParseFailure> {
        let map = |range: Range<usize>| -> Range<usize> {
            let find = |at: usize| parts.iter().rev().find(|part| part.0 <= at).copied();
            let start = find(range.start).map_or(0, |(from, source, length)| {
                source + (range.start - from).min(length)
            });
            let end = find(range.end.saturating_sub(1).max(range.start))
                .map_or(start, |(from, source, length)| {
                    source + (range.end - from).min(length)
                });
            start..end.max(start)
        };
        let mut open: Vec<bool> = Vec::new();
        let mut last_end = 0;
        for op in ops {
            match op {
                html::Op::Open { kind, align, range } => {
                    let block = matches!(kind, NodeKind::Paragraph | NodeKind::Heading { .. });
                    let range = map(range);
                    last_end = last_end.max(range.end);
                    self.start(kind, range)?;
                    if block {
                        self.mark_alignment(align);
                    }
                    open.push(block);
                }
                html::Op::Close => {
                    let block = open.pop().unwrap_or(false);
                    let id = self.finish_node()?;
                    let node = &mut self.document.nodes[id.0];
                    node.source.end = SourceByte(last_end.max(node.source.start.0));
                    if block {
                        self.newline(last_end)?;
                    }
                }
                html::Op::Text { text, range } => {
                    let range = map(range);
                    last_end = last_end.max(range.end);
                    self.leaf(NodeKind::Text, &text, range, false)?;
                }
                html::Op::Code { text, range } => {
                    let range = map(range);
                    last_end = last_end.max(range.end);
                    self.leaf(NodeKind::Code, &text, range, false)?;
                }
                html::Op::Image {
                    destination,
                    title,
                    alt,
                    align,
                    range,
                } => {
                    let range = map(range);
                    last_end = last_end.max(range.end);
                    self.start(NodeKind::Image { destination, title }, range.clone())?;
                    self.mark_alignment(align);
                    self.leaf(NodeKind::Text, &alt, range, false)?;
                    self.finish_node()?;
                }
                html::Op::HardBreak { range } => {
                    let range = map(range);
                    last_end = last_end.max(range.end);
                    self.leaf(NodeKind::HardBreak, "\n", range, false)?;
                }
                html::Op::Rule { range } => {
                    let range = map(range);
                    last_end = last_end.max(range.end);
                    self.newline(range.start)?;
                    self.leaf(NodeKind::ThematicBreak, "", range.clone(), true)?;
                    self.newline(range.end)?;
                }
            }
        }
        while open.pop().is_some() {
            self.finish_node()?;
        }
        Ok(())
    }
    fn budget(&self) -> Result<(), ParseFailure> {
        // Includes capacity growth, snapshots, arena links, attributes and the
        // eventual reverse mapping index. It does not bound upstream first-pass memory.
        let estimated = self.document.source.capacity()
            + self.document.display_text.capacity()
            + self.document.nodes.capacity() * std::mem::size_of::<Node>()
            + self.attribute_bytes
            + self.document.mapping.capacity()
                * (std::mem::size_of::<MappingSegment>() + std::mem::size_of::<usize>())
            + self.document.headings.capacity() * std::mem::size_of::<Heading>()
            + self.document.diagnostics.capacity() * std::mem::size_of::<Diagnostic>()
            + self.document.alignments.capacity()
                * std::mem::size_of::<(NodeId, ColumnAlignment)>();
        if estimated > self.limits.retained_bytes {
            Err(self.failure(ParseStop::RetainedBudget))
        } else {
            Ok(())
        }
    }
    fn finish(mut self) -> Result<MarkdownDocument, ParseFailure> {
        if self.stack.len() != 1 {
            return Err(self.failure(ParseStop::InvalidStructure));
        }
        self.document.nodes[0].display.end = DisplayByte(self.document.display_text.len());
        self.document.source_order = (0..self.document.mapping.len()).collect();
        self.document.source_order.sort_unstable_by_key(|i| {
            (
                self.document.mapping[*i].source.start,
                self.document.mapping[*i].source.end,
            )
        });
        self.budget()?;
        Ok(self.document)
    }
}

pub fn parse_document(
    source: Arc<String>,
    format: ContentFormat,
    syntax: SyntaxOptions,
    limits: ParseLimits,
    mut cancelled: impl FnMut() -> bool,
) -> Result<MarkdownDocument, ParseFailure> {
    let failure = |reason| ParseFailure {
        reason,
        progress: ParseProgress::default(),
    };
    if source.len() > limits.source_bytes {
        return Err(failure(ParseStop::SourceBudget));
    }
    if cancelled() {
        return Err(failure(ParseStop::Cancelled));
    }
    let mut out = Builder::new(source.clone(), format, limits);
    if format == ContentFormat::PlainText {
        out.leaf(NodeKind::Text, source.as_str(), 0..source.len(), false)?;
        return out.finish();
    }
    let mut options = Options::empty();
    if syntax.tables {
        options.insert(Options::ENABLE_TABLES);
    }
    if syntax.tasks {
        options.insert(Options::ENABLE_TASKLISTS);
    }
    if syntax.strikethrough {
        options.insert(Options::ENABLE_STRIKETHROUGH);
    }
    // pulldown's constructor has no cancellation hook. Admission happens before
    // construction; cancellation is checked again before every projected event.
    let parser = Parser::new_ext(source.as_str(), options);
    let mut html_block: Option<HtmlBlock> = None;
    for (event, span) in parser.into_offset_iter() {
        if cancelled() {
            return Err(out.failure(ParseStop::Cancelled));
        }
        out.document.progress.events += 1;
        if out.document.progress.events > limits.events {
            return Err(out.failure(ParseStop::EventBudget));
        }
        match event {
            Event::Start(tag) => match tag {
                Tag::Paragraph => out.start(NodeKind::Paragraph, span)?,
                Tag::Heading { level, .. } => {
                    out.start(NodeKind::Heading { level: level as u8 }, span)?
                }
                Tag::BlockQuote(_) => out.start(NodeKind::Quote, span)?,
                Tag::CodeBlock(kind) => {
                    let (language, fenced) = match kind {
                        CodeBlockKind::Fenced(info) => {
                            (info.split_whitespace().next().map(str::to_owned), true)
                        }
                        CodeBlockKind::Indented => (None, false),
                    };
                    out.start(NodeKind::CodeBlock { language, fenced }, span)?;
                }
                // Collected whole: the supported subset becomes real nodes,
                // anything else stays literal when the block ends.
                Tag::HtmlBlock => html_block = Some((span, Vec::new())),
                Tag::List(start) => {
                    out.newline(span.start)?;
                    out.start(NodeKind::List { start }, span)?;
                }
                Tag::Item => {
                    out.newline(span.start)?;
                    let ordinal = out
                        .stack
                        .iter_mut()
                        .rev()
                        .find(|frame| {
                            matches!(out.document.nodes[frame.node.0].kind, NodeKind::List { .. })
                        })
                        .and_then(|frame| {
                            let n = frame.next_ordinal;
                            frame.next_ordinal = n.map(|v| v.saturating_add(1));
                            n
                        });
                    let depth = out
                        .stack
                        .iter()
                        .filter(|frame| {
                            matches!(out.document.nodes[frame.node.0].kind, NodeKind::List { .. })
                        })
                        .count()
                        .saturating_sub(1);
                    out.start(
                        NodeKind::ListItem {
                            ordinal,
                            task: None,
                        },
                        span.clone(),
                    )?;
                    let marker = format!(
                        "{}{}",
                        "  ".repeat(depth),
                        ordinal.map_or_else(|| "• ".to_owned(), |n| format!("{n}. "))
                    );
                    out.push(&marker, span.start..span.start, true)?;
                }
                Tag::Table(columns) => {
                    out.newline(span.start)?;
                    let columns = columns
                        .into_iter()
                        .map(|a| match a {
                            Alignment::None => ColumnAlignment::None,
                            Alignment::Left => ColumnAlignment::Left,
                            Alignment::Center => ColumnAlignment::Center,
                            Alignment::Right => ColumnAlignment::Right,
                        })
                        .collect();
                    out.start(NodeKind::Table { columns }, span)?;
                }
                Tag::TableHead => out.start(NodeKind::TableRow { header: true }, span)?,
                Tag::TableRow => out.start(NodeKind::TableRow { header: false }, span)?,
                Tag::TableCell => {
                    let previous = out.stack.last().map_or(0, |frame| frame.cells);
                    if previous > 0 {
                        out.push("\t", span.start..span.start, true)?;
                    }
                    if let Some(frame) = out.stack.last_mut() {
                        frame.cells += 1;
                    }
                    out.start(NodeKind::TableCell, span)?;
                }
                Tag::Emphasis => out.start(NodeKind::Emphasis, span)?,
                Tag::Strong => out.start(NodeKind::Strong, span)?,
                Tag::Strikethrough => out.start(NodeKind::Strikethrough, span)?,
                Tag::Link {
                    dest_url, title, ..
                } => out.start(
                    NodeKind::Link {
                        destination: dest_url.into_string(),
                        title: title.into_string(),
                    },
                    span,
                )?,
                Tag::Image {
                    dest_url, title, ..
                } => out.start(
                    NodeKind::Image {
                        destination: dest_url.into_string(),
                        title: title.into_string(),
                    },
                    span,
                )?,
                _ => {
                    out.document.diagnostics.push(Diagnostic {
                        kind: DiagnosticKind::UnsupportedSyntax,
                        source: SourceByte(span.start)..SourceByte(span.end),
                    });
                    out.start(NodeKind::Unsupported, span)?;
                }
            },
            Event::End(TagEnd::HtmlBlock) => {
                let (block, parts) = html_block.take().unwrap_or((span.clone(), Vec::new()));
                let mut text = String::new();
                let mut offsets = Vec::with_capacity(parts.len());
                for (part, range) in &parts {
                    offsets.push((text.len(), range.start, part.len()));
                    text.push_str(part);
                }
                match html::plan(&text) {
                    Some(ops) => out.apply_html(ops, &offsets)?,
                    None => {
                        out.document.diagnostics.push(Diagnostic {
                            kind: DiagnosticKind::LiteralHtml,
                            source: SourceByte(block.start)..SourceByte(block.end),
                        });
                        out.start(NodeKind::Html { block: true }, block)?;
                        for (part, range) in parts {
                            out.leaf(NodeKind::Html { block: false }, &part, range, false)?;
                        }
                        let id = out.finish_node()?;
                        out.newline(out.document.nodes[id.0].source.end.0)?;
                    }
                }
            }
            Event::End(tag) => {
                let id = out.finish_node()?;
                if matches!(
                    tag,
                    TagEnd::Paragraph
                        | TagEnd::Heading(_)
                        | TagEnd::CodeBlock
                        | TagEnd::HtmlBlock
                        | TagEnd::Item
                        | TagEnd::List(_)
                        | TagEnd::BlockQuote(_)
                        | TagEnd::TableHead
                        | TagEnd::TableRow
                        | TagEnd::Table
                ) {
                    out.newline(out.document.nodes[id.0].source.end.0)?;
                }
            }
            Event::Text(text) => out.leaf(NodeKind::Text, &text, span, false)?,
            Event::Code(text) => out.leaf(NodeKind::Code, &text, span, false)?,
            Event::Html(text) if html_block.is_some() => {
                if let Some((_, parts)) = html_block.as_mut() {
                    parts.push((text.into_string(), span));
                }
            }
            Event::Html(text) | Event::InlineHtml(text) => {
                out.leaf(NodeKind::Html { block: false }, &text, span, false)?
            }
            Event::SoftBreak => out.leaf(NodeKind::SoftBreak, " ", span, false)?,
            Event::HardBreak => out.leaf(NodeKind::HardBreak, "\n", span, false)?,
            Event::Rule => {
                out.newline(span.start)?;
                out.leaf(NodeKind::ThematicBreak, "", span.clone(), true)?;
                out.newline(span.end)?;
            }
            Event::TaskListMarker(done) => {
                for frame in out.stack.iter().rev() {
                    if let NodeKind::ListItem { task, .. } =
                        &mut out.document.nodes[frame.node.0].kind
                    {
                        *task = Some(done);
                        break;
                    }
                }
                out.push(if done { "[x] " } else { "[ ] " }, span, false)?;
            }
            _ => {
                let text = source.get(span.clone()).unwrap_or("");
                out.leaf(NodeKind::Unsupported, text, span, false)?;
            }
        }
        out.budget()?;
    }
    if cancelled() {
        return Err(out.failure(ParseStop::Cancelled));
    }
    out.finish()
}
