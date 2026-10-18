# html

- Grammar crate: tree-sitter-html 0.23.2, locked in Cargo.lock.
- Upstream: https://github.com/tree-sitter/tree-sitter-html
- Upstream commit of the crate release: `5a5ca8551a179998360b4a4ca2c0f366a35acc03`
- License: MIT
- Query: queries/highlights.scm and queries/injections.scm of the same crate version.
- Modified: No.

The query uses the capture names, predicates and priority rule described in
`src/features/syntax/queries.rs`; injection queries follow
`src/features/syntax/injections.rs`. Update this file together with the crate
version in Cargo.toml, and run the language fixtures and the query tests.
