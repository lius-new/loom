//! A plain text buffer with a byte-offset cursor.
//!
//! Kept entirely free of UI types so it can be unit-tested in isolation and
//! swapped for a rope/CRDT later without touching the editor view.

#[derive(Clone, Debug)]
pub struct TextBuffer {
    text: String,
    cursor: usize, // byte offset into `text`
}

impl TextBuffer {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            cursor: 0,
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn set_cursor(&mut self, pos: usize) {
        self.cursor = pos.min(self.text.len());
    }

    // ---- Editing -------------------------------------------------------

    pub fn insert(&mut self, s: &str) {
        self.text.insert_str(self.cursor, s);
        self.cursor += s.len();
    }

    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        if let Some(c) = self.text[..self.cursor].chars().next_back() {
            let prev = self.cursor - c.len_utf8();
            self.text.replace_range(prev..self.cursor, "");
            self.cursor = prev;
        }
    }

    pub fn delete_forward(&mut self) {
        if self.cursor >= self.text.len() {
            return;
        }
        if let Some(c) = self.text[self.cursor..].chars().next() {
            self.text.replace_range(self.cursor..self.cursor + c.len_utf8(), "");
        }
    }

    // ---- Navigation ----------------------------------------------------

    pub fn move_left(&mut self) {
        if self.cursor == 0 {
            return;
        }
        if let Some(c) = self.text[..self.cursor].chars().next_back() {
            self.cursor -= c.len_utf8();
        }
    }

    pub fn move_right(&mut self) {
        if self.cursor >= self.text.len() {
            return;
        }
        if let Some(c) = self.text[self.cursor..].chars().next() {
            self.cursor += c.len_utf8();
        }
    }

    pub fn move_up(&mut self) {
        let (line, col) = self.line_col();
        if line == 0 {
            return;
        }
        let bounds = self.line_bounds();
        let (s, e) = bounds[line - 1];
        let target = col.min(self.text[s..e].chars().count());
        self.cursor = s + char_index_to_byte(&self.text[s..e], target);
    }

    pub fn move_down(&mut self) {
        let (line, col) = self.line_col();
        let bounds = self.line_bounds();
        if line + 1 >= bounds.len() {
            return;
        }
        let (s, e) = bounds[line + 1];
        let target = col.min(self.text[s..e].chars().count());
        self.cursor = s + char_index_to_byte(&self.text[s..e], target);
    }

    pub fn move_home(&mut self) {
        let (line, _) = self.line_col();
        let bounds = self.line_bounds();
        self.cursor = bounds[line].0;
    }

    pub fn move_end(&mut self) {
        let (line, _) = self.line_col();
        let bounds = self.line_bounds();
        self.cursor = bounds[line].1;
    }

    // ---- Queries -------------------------------------------------------

    /// 0-based (line, column-in-chars) of the cursor.
    pub fn line_col(&self) -> (usize, usize) {
        let mut line = 0;
        let mut col = 0;
        for (i, c) in self.text.char_indices() {
            if i >= self.cursor {
                break;
            }
            if c == '\n' {
                line += 1;
                col = 0;
            } else {
                col += 1;
            }
        }
        (line, col)
    }

    pub fn line_count(&self) -> usize {
        self.line_bounds().len()
    }

    /// Byte ranges `(start, end)` of every line (excluding the newline).
    pub fn line_bounds(&self) -> Vec<(usize, usize)> {
        let mut lines = Vec::new();
        let mut start = 0;
        for (i, c) in self.text.char_indices() {
            if c == '\n' {
                lines.push((start, i));
                start = i + 1;
            }
        }
        lines.push((start, self.text.len()));
        lines
    }

    /// The text of the line at `idx` (clamped to the last line).
    pub fn line(&self, idx: usize) -> &str {
        let bounds = self.line_bounds();
        let (s, e) = bounds[idx.min(bounds.len() - 1)];
        &self.text[s..e]
    }
}

fn char_index_to_byte(s: &str, n: usize) -> usize {
    s.char_indices()
        .nth(n)
        .map(|(i, _)| i)
        .unwrap_or(s.len())
}
