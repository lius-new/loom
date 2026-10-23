//! Outline items from a server's document symbols.
use super::model::{Item, Kind, nest};
use crate::features::language_service::position::LineIndex;
use crate::features::language_service::reply::Symbol;

/// Converts symbols read against `text` into items. Hierarchical answers keep
/// their shape; flat ones are nested by the range that covers each symbol.
pub fn items(symbols: &[Symbol], text: &str) -> Vec<Item> {
    let index = LineIndex::new(text);
    let flat = !symbols.is_empty() && symbols.iter().all(|symbol| symbol.uri.is_some());
    let items = symbols
        .iter()
        .map(|symbol| convert(symbol, &index))
        .collect::<Vec<_>>();
    if flat { nest(items) } else { items }
}

fn convert(symbol: &Symbol, index: &LineIndex<'_>) -> Item {
    let range = |span: &crate::features::language_service::reply::Span| {
        let start = index.offset(span.start, span.encoding);
        let end = index.offset(span.end, span.encoding);
        start.min(end)..start.max(end)
    };
    let full = range(&symbol.full);
    let mut select = range(&symbol.selection);
    if select.start < full.start || select.end > full.end {
        select = full.start..full.start;
    }
    Item {
        name: symbol.name.clone(),
        detail: symbol.detail.clone().filter(|detail| !detail.is_empty()),
        kind: Kind::from_lsp(symbol.kind),
        deprecated: symbol.deprecated,
        full,
        select,
        children: symbol
            .children
            .iter()
            .map(|child| convert(child, index))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::language_service::capabilities::PositionEncoding;
    use crate::features::language_service::reply;
    use serde_json::json;

    #[test]
    fn hierarchical_document_symbols_keep_their_shape_and_land_on_the_name() {
        let text = "struct A {\n    b: u8,\n}\n";
        let value = json!([{
            "name": "A", "kind": 23, "detail": "",
            "range": {"start": {"line": 0, "character": 0}, "end": {"line": 2, "character": 1}},
            "selectionRange": {"start": {"line": 0, "character": 7}, "end": {"line": 0, "character": 8}},
            "children": [{
                "name": "b", "kind": 8, "detail": "u8",
                "range": {"start": {"line": 1, "character": 4}, "end": {"line": 1, "character": 9}},
                "selectionRange": {"start": {"line": 1, "character": 4}, "end": {"line": 1, "character": 5}}
            }]
        }]);
        let symbols = reply::symbols(&value, PositionEncoding::Utf16);
        let items = items(&symbols, text);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].kind, Kind::Struct);
        assert_eq!(items[0].select, 7..8);
        assert_eq!(items[0].detail, None);
        assert_eq!(items[0].children[0].detail.as_deref(), Some("u8"));
        assert_eq!(&text[items[0].children[0].select.clone()], "b");
    }

    #[test]
    fn flat_symbol_information_is_nested_by_range() {
        let text = "class A {\n  m() {}\n}\n";
        let location = |l0: u32, c0: u32, l1: u32, c1: u32| {
            json!({"uri": "file:///a.ts", "range": {
                "start": {"line": l0, "character": c0}, "end": {"line": l1, "character": c1}}})
        };
        let value = json!([
            {"name": "m", "kind": 6, "location": location(1, 2, 1, 8), "containerName": "A"},
            {"name": "A", "kind": 5, "location": location(0, 0, 2, 1)},
        ]);
        let symbols = reply::symbols(&value, PositionEncoding::Utf16);
        let items = items(&symbols, text);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "A");
        assert_eq!(items[0].children[0].name, "m");
    }
}
