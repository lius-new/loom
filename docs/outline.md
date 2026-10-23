# Outline

Click the structure icon in the right drawer (next to Explorer and Search), or
run `workspace::OpenOutline`, to see the structure of the current file as a tree.
It works for any text file, not only Markdown.

## Where the tree comes from

- **Markdown files** (`.md`, `.markdown`, `.mdown`, `.mkd`): headings, nested by
  level. Fenced code and front matter are skipped. This is read directly from the
  text, so it needs no language server.
- **Other files:** the document symbols that the running language servers report,
  for example classes, methods and fields. Servers that answer with a flat list
  are nested by the range each symbol covers. If no server is ready yet the page
  asks again a few times; otherwise it says that no structure is available.

The tree refreshes 300 ms after typing pauses, and an old tree stays visible
until the new one arrives. Switching files replaces it at once.

## Using the tree

Click a row to move the caret to that symbol. The chevron expands or collapses a
branch. The row that contains the caret is marked and scrolled into view as you
move through the file; if it sits inside a collapsed branch, that branch is
marked instead. The toolbar has **Expand All**, **Collapse All** and **Sort
Alphabetically**. Collapsed branches and sorting survive edits.

Icons follow the editor's syntax colors: classes, interfaces and enums, methods
and functions, fields and properties, constants, namespaces, and so on. Headings
show an `H1`–`H6` tag. A symbol the server marks deprecated is dimmed.

The scrollbar, hover and resize handle match the Explorer and editor.

## Limits

The outline of a preview tab is not shown; focus the source tab of that file.
