# rust

- Grammar reference: tree-sitter-rust 0.24.2, upstream reference version; native modules are supplied externally.
- Upstream: https://github.com/tree-sitter/tree-sitter-rust
- Upstream reference commit: `e2bee853694a1d3e0f6ef308fe3674542fec95d7`
- Verified standard WASM: https://github.com/tree-sitter/tree-sitter-rust/releases/download/v0.24.2/tree-sitter-rust.wasm
- WASM SHA-256: `24c89bd9252255e4aebbcbd7d2d308bd92c86dd95a130fdc80efa49577b8d738`
- License: MIT, Copyright (c) 2017 Maxim Sokolov
- Query: queries/highlights.scm of the same upstream version.
- Modified: No.

The query uses the capture names, predicates and priority rule described in
`src/features/syntax/queries.rs`. Update this file together with the upstream module
version/commit, and run the language fixtures and the query tests.
