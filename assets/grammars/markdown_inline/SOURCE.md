# markdown_inline

- Grammar crate: tree-sitter-md 0.5.3 (inline grammar), locked in Cargo.lock.
- Upstream: https://github.com/tree-sitter-grammars/tree-sitter-markdown
- Upstream commit of the crate release: `f969cd3ae3f9fbd4e43205431d0ae286014c05b5`
- License: MIT
- Query: written for Loom from tree-sitter-markdown-inline/queries/highlights.scm of the same crate version.
- Modified: Yes: captures renamed to Loom's classes.

The query uses the capture names, predicates and priority rule described in
`src/features/syntax/queries.rs`; injection queries follow
`src/features/syntax/injections.rs`. Update this file together with the crate
version in Cargo.toml, and run the language fixtures and the query tests.
