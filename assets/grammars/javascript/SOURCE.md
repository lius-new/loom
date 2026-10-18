# javascript

- Grammar crate: tree-sitter-javascript 0.25.0, locked in Cargo.lock.
- Upstream: https://github.com/tree-sitter/tree-sitter-javascript
- Upstream commit of the crate release: `44c892e0be055ac465d5eeddae6d3e194424e7de`
- License: MIT, Copyright (c) 2014 Max Brunsfeld
- Query: queries/highlights.scm followed by queries/highlights-jsx.scm of the same crate version.
- Modified: Concatenated only. Loom has one JavaScript variant for .js and .jsx files, so the JSX patterns are always part of the query.

The query uses the capture names, predicates and priority rule described in
`src/features/syntax/queries.rs`. Update this file together with the crate
version in Cargo.toml, and run the language fixtures and the query tests.
