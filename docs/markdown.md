# Markdown reading

Loom previews the current buffer, including unsaved changes. Source and reading
views share one document, saved state and undo history. Their positions,
selections and scroll offsets are independent.

## Opening a preview

Open a .md, .markdown, .mdown or .mkd file and use:

| Action | Default shortcut | Behavior |
| --- | --- | --- |
| markdown::TogglePreview | Ctrl+Shift+V | Toggle the current tab between source and reading |
| markdown::OpenPreviewToSide | Ctrl+K, V | Open or focus the explicitly associated side preview |
| markdown::OpenSource | Configurable | Focus the associated source view |
| markdown::PreviewLargerDocument | Configurable | Explicitly raise the source admission limit to 2 MiB |

Use Command instead of Ctrl on macOS. The tab context menu also provides these
actions. **Open Source** in the reader toolbar locates the current reading
anchor in source. Repeating side preview reuses its association. Copying a tab
creates a separate view; moving it preserves its identity. Closing or converting
the associated source safely detaches the preview.

The reader toolbar provides **Outline**, **Sync Scroll**, **Load Remote Images**
and **Larger Preview**. In a narrow pane, scroll the toolbar horizontally to
reach the remaining controls. Outline entries navigate by source anchors. Sync Scroll
follows the focused member of an associated source/preview pair, preventing a
feedback loop. Session restore keeps view modes, positions and associations.

## Reading and copying

Drag to select, including beyond the visible page. Holding a drag outside the
viewport continues scrolling until release. Copy reads the complete logical
selection; Select All includes text outside the scene. Code blocks preserve
whitespace and offer a **Copy** button. Wide code and tables have horizontal
scrollbars; horizontal wheel movement or Shift+wheel scrolls the block.

Arrow keys move the selection by grapheme/visual line; Shift extends it.
Home/End go to the document edges, Page Up/Down scroll, Escape clears selection,
and Ctrl/Command+A/C select all/copy. Tab and Shift+Tab visit document links;
Enter opens the selected link. Tab leaves the reader at the end of the links.
Text input, paste, Vim edits and IME composition cannot edit the reading surface.
Save still saves the shared document.

Hover, completion and signature documentation use the same parser, paragraph
adapter and logical reader. LSP plaintext stays literal. A card's source label
or diagnostic metadata has its own selection; its Markdown documentation is a
separate logical region. Cards do not load images or other external resources.

## Supported content

| Content | Current behavior |
| --- | --- |
| CommonMark | Paragraphs, headings, emphasis, strong, links, code, quotes, ordered/unordered lists and thematic breaks |
| Explicit extensions | Tables, task list markers and strikethrough |
| Chinese, combining marks, Emoji, tabs | Whole paragraph native shaping; grapheme-safe selection; four-column code tabs |
| HTML | Displayed literally, including tags; never executed |
| PNG/JPEG | Local images relative to the document; bounded decoding; alternative text and visible failure state |
| Inline images | Promoted to separate flow objects; alt text remains in logical copy |
| Links | HTTP(S), local files and percent-encoded heading fragments; local files open through the normal file loader |
| Fenced code highlighting | Visible blocks request existing language packages; unknown or unavailable grammars remain readable |
| SVG, math and Mermaid | No active renderer; image fallback or literal syntax |

Remote images are off by default. **Load Remote Images** authorizes only the
current document path for the current runtime view; it is not saved as a global
or session permission. UNC paths follow the same opt-in policy. Local
image changes invalidate their cached dimensions and pixels on a one-second
poll. Remote loads use bounded byte/time limits. Unsupported links report an
error instead of launching an arbitrary protocol.

## Preferences and limits

Add this to user or project settings:

```json
{
  "markdown": {"font_size": 16, "line_height": 22, "max_width": 960}
}
```

Text uses system font fallback. Install Noto CJK and Noto Color Emoji on Linux
for Chinese and color Emoji coverage. Font size accepts 10–32, line height 14–80, and maximum reading width 320–1800
logical pixels. Effective line height is at least 1.2 times the font size.

Automatic preview admits source up to 512 KiB; Larger Preview admits 2 MiB.
Event, depth, output and retained-memory limits also apply. A native paragraph
is limited to 16 KiB. Oversized blocks/cells show an explicit fallback while
other content remains readable; Open Source displays the full source and
logical Select All/Copy retains the block. Other parse/layout budget failures
show a diagnostic rather than silently truncating content.

Images accept at most 8 MiB encoded data, 4096 pixels per side and 16 MiB
decoded RGBA. These retained budgets do not promise a process RSS or native
temporary-allocation ceiling. See [the architecture](architecture/markdown.md)
for lifecycle, extension points and reproducible validation.
