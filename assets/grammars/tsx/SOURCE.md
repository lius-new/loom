# tsx

- Grammar crate: tree-sitter-typescript 0.23.2 (LANGUAGE_TSX), locked in Cargo.lock.
- Upstream: https://github.com/tree-sitter/tree-sitter-typescript
- Upstream commit of the crate release: `f975a621f4e7f532fe322e13c4f79495e0a7b2e7`
- License: MIT (as declared by the crate)
- Query: queries/highlights.scm and queries/highlights-jsx.scm of tree-sitter-javascript 0.23.1 (3a837b6f3658ca3618f2022f8707e29739c91364) followed by queries/highlights.scm of tree-sitter-typescript 0.23.2.
- Modified: Concatenated only, for the same reason as the typescript grammar, plus the JSX patterns.

The query uses the capture names, predicates and priority rule described in
`src/features/syntax/queries.rs`. Update this file together with the crate
version in Cargo.toml, and run the language fixtures and the query tests.
