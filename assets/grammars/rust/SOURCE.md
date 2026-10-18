# rust

- Grammar crate: tree-sitter-rust 0.24.2, locked in Cargo.lock.
- Upstream: https://github.com/tree-sitter/tree-sitter-rust
- Upstream commit of the crate release: `e2bee853694a1d3e0f6ef308fe3674542fec95d7`
- License: MIT, Copyright (c) 2017 Maxim Sokolov
- Query: queries/highlights.scm of the same crate version.
- Modified: No.

The query uses the capture names, predicates and priority rule described in
`src/features/syntax/queries.rs`. Update this file together with the crate
version in Cargo.toml, and run the language fixtures and the query tests.
