# javascript

- Grammar reference: tree-sitter-javascript 0.25.0, upstream reference version; native modules are supplied externally.
- Upstream: https://github.com/tree-sitter/tree-sitter-javascript
- Upstream reference commit: `44c892e0be055ac465d5eeddae6d3e194424e7de`
- License: MIT, Copyright (c) 2014 Max Brunsfeld
- Query: queries/highlights.scm followed by queries/highlights-jsx.scm of the same upstream version.
- Modified: Concatenated only. Loom has one JavaScript variant for .js and .jsx files, so the JSX patterns are always part of the query.

The query uses the capture names, predicates and priority rule described in
`src/features/syntax/queries.rs`. Update this file together with the upstream module
version/commit, and run the language fixtures and the query tests.
