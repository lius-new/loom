# html

- Grammar reference: tree-sitter-html 0.23.2, upstream reference version; native modules are supplied externally.
- Upstream: https://github.com/tree-sitter/tree-sitter-html
- Upstream reference commit: `5a5ca8551a179998360b4a4ca2c0f366a35acc03`
- License: MIT
- Query: queries/highlights.scm and queries/injections.scm of the same upstream version.
- Modified: No.

The query uses the capture names, predicates and priority rule described in
`src/features/syntax/queries.rs`; injection queries follow
`src/features/syntax/injections.rs`. Update this file together with the upstream module
version/commit, and run the language fixtures and the query tests.
