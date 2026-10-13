//! Resolve a navigation intent using an immutable snapshot or injected disk source.
use super::{Matcher, model::NavigationIntent, ports::SearchSourcePort};
use crate::domain::document_snapshot::DocumentSnapshot;
use std::ops::Range;
use std::sync::Arc;

pub fn prepare(
    source: &dyn SearchSourcePort,
    intent: &NavigationIntent,
    buffer: Option<&DocumentSnapshot>,
) -> Result<(Arc<String>, Range<usize>), String> {
    let found = intent
        .file
        .matches
        .get(intent.index)
        .ok_or_else(|| "Search result is no longer available.".to_owned())?;
    let text = match buffer {
        Some(buffer) => buffer.text.clone(),
        None => Arc::new(source.read_text(&intent.file.path)?),
    };
    let range = Matcher::new(&intent.spec)?
        .locate(&text, &intent.file, found)
        .ok_or_else(|| "File changed; searching again to update this result.".to_owned())?;
    Ok((text, range))
}
