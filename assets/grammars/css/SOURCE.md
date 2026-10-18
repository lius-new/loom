# css

- Grammar crate: tree-sitter-css 0.25.0, locked in Cargo.lock.
- Upstream: https://github.com/tree-sitter/tree-sitter-css
- Upstream commit of the crate release: `dda5cfc5722c429eaba1c910ca32c2c0c5bb1a3f`
- License: MIT
- Query: queries/highlights.scm of the same crate version.
- Modified: No.

The query uses the capture names, predicates and priority rule described in
`src/features/syntax/queries.rs`; injection queries follow
`src/features/syntax/injections.rs`. Update this file together with the crate
version in Cargo.toml, and run the language fixtures and the query tests.
