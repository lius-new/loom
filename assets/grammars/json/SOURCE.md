# json

- Grammar crate: tree-sitter-json 0.24.8, locked in Cargo.lock.
- Upstream: https://github.com/tree-sitter/tree-sitter-json
- Upstream commit of the crate release: `ee35a6ebefcef0c5c416c0d1ccec7370cfca5a24`
- License: MIT
- Query: queries/highlights.scm of the same crate version.
- Modified: Yes: object keys are `@property` instead of `@string.special.key`.

The query uses the capture names, predicates and priority rule described in
`src/features/syntax/queries.rs`; injection queries follow
`src/features/syntax/injections.rs`. Update this file together with the crate
version in Cargo.toml, and run the language fixtures and the query tests.
