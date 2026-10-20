// Compose the real production module with package-dependent tests externally.
include!("../../../src/features/syntax/mod.rs");

pub(crate) fn validate_producer_grammar(
    resources: &dyn Resources,
    name: &str,
    mut parser: tree_sitter::Parser,
    text: &str,
) -> Result<usize, String> {
    use streaming_iterator::StreamingIterator;
    let query = queries::compile(resources, name)?;
    injections::compile(resources, name)?;
    let tree = parser.parse(text, None).ok_or("parse cancelled")?;
    if tree.root_node().has_error() {
        return Err(format!(
            "{name}: validation sample has parse errors: {}",
            tree.root_node().to_sexp()
        ));
    }
    let mut cursor = tree_sitter::QueryCursor::new();
    let mut captures = cursor.captures(&query.query, tree.root_node(), text.as_bytes());
    let mut count = 0;
    while let Some((matched, index)) = captures.next() {
        if query.class_of(matched.captures()[*index].index).is_some() {
            count += 1;
        }
    }
    if count == 0 {
        return Err(format!(
            "{name}: validation sample has no highlight captures"
        ));
    }
    Ok(count)
}
#[cfg(test)]
#[path = "../suites/injection_tests.rs"]
mod injection_tests;
#[cfg(test)]
#[path = "../suites/test_support.rs"]
pub(crate) mod test_support;
#[cfg(test)]
#[path = "../suites/treesitter_bench.rs"]
mod treesitter_bench;
#[cfg(test)]
#[path = "../suites/treesitter_tests.rs"]
mod treesitter_tests;

#[cfg(test)]
#[path = "../suites/presentation_integration_tests.rs"]
mod presentation_integration_tests;
