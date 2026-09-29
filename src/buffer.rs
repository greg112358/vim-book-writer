use std::ops::Range;

/// Text with a cursor. The cursor is a byte offset that always sits on a char boundary.
/// Every edit goes through `replace`, so undo can later hook in at one place.
#[derive(Debug, Default)]
pub struct Buffer {
    text: String,
    cursor: usize,
    /// Column (in chars) that up/down aim for, so a short line doesn't lose the column.
    goal_col: Option<usize>,
}

impl Buffer {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            ..Self::default()
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Moves the cursor to `pos`, clamped to the text and rounded down to a char boundary.
    pub fn set_cursor(&mut self, pos: usize) {
        let mut pos = pos.min(self.text.len());
        while !self.text.is_char_boundary(pos) {
            pos -= 1;
        }
        self.cursor = pos;
        self.goal_col = None;
    }

    /// Replaces `range` with `with` and puts the cursor after the new text.
    /// Panics if `range` is out of bounds or not on char boundaries.
    pub fn replace(&mut self, range: Range<usize>, with: &str) {
        self.cursor = range.start + with.len();
        self.text.replace_range(range, with);
        self.goal_col = None;
    }

    pub fn insert(&mut self, text: &str) {
        self.replace(self.cursor..self.cursor, text);
    }

    pub fn delete_back(&mut self) {
        if let Some(c) = self.text[..self.cursor].chars().next_back() {
            self.replace(self.cursor - c.len_utf8()..self.cursor, "");
        }
    }

    pub fn delete_forward(&mut self) {
        if let Some(c) = self.text[self.cursor..].chars().next() {
            self.replace(self.cursor..self.cursor + c.len_utf8(), "");
        }
    }

    /// Moves one char left, crossing line breaks.
    pub fn move_left(&mut self) {
        if let Some(c) = self.text[..self.cursor].chars().next_back() {
            self.set_cursor(self.cursor - c.len_utf8());
        }
    }

    /// Moves one char right, crossing line breaks.
    pub fn move_right(&mut self) {
        if let Some(c) = self.text[self.cursor..].chars().next() {
            self.set_cursor(self.cursor + c.len_utf8());
        }
    }

    pub fn line_start(&self) -> usize {
        self.text[..self.cursor].rfind('\n').map_or(0, |i| i + 1)
    }

    pub fn line_end(&self) -> usize {
        self.text[self.cursor..]
            .find('\n')
            .map_or(self.text.len(), |i| self.cursor + i)
    }

    pub fn move_line_start(&mut self) {
        self.set_cursor(self.line_start());
    }

    pub fn move_line_end(&mut self) {
        self.set_cursor(self.line_end());
    }

    /// Moves to the line above at the goal column. Does nothing on the first line.
    pub fn move_up(&mut self) {
        let start = self.line_start();
        if start == 0 {
            return;
        }
        let prev_start = self.text[..start - 1].rfind('\n').map_or(0, |i| i + 1);
        self.move_to_line(prev_start, start - 1);
    }

    /// Moves to the line below at the goal column. Does nothing on the last line.
    pub fn move_down(&mut self) {
        let end = self.line_end();
        if end == self.text.len() {
            return;
        }
        let next_end = self.text[end + 1..]
            .find('\n')
            .map_or(self.text.len(), |i| end + 1 + i);
        self.move_to_line(end + 1, next_end);
    }

    fn move_to_line(&mut self, start: usize, end: usize) {
        let col = self
            .goal_col
            .unwrap_or_else(|| self.text[self.line_start()..self.cursor].chars().count());
        let line = &self.text[start..end];
        self.cursor = start + line.char_indices().nth(col).map_or(line.len(), |(i, _)| i);
        self.goal_col = Some(col);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_and_moves_by_whole_chars() {
        let mut buf = Buffer::default();
        buf.insert("héllo");
        assert_eq!(buf.cursor(), 6);
        buf.delete_back();
        buf.move_left();
        buf.move_left();
        buf.move_left();
        buf.delete_forward();
        assert_eq!((buf.text(), buf.cursor()), ("hll", 1));
    }

    #[test]
    fn edges_are_no_ops() {
        let mut buf = Buffer::new("ab");
        buf.delete_back();
        buf.move_left();
        buf.move_up();
        assert_eq!((buf.text(), buf.cursor()), ("ab", 0));
        buf.set_cursor(99);
        buf.delete_forward();
        buf.move_right();
        buf.move_down();
        assert_eq!((buf.text(), buf.cursor()), ("ab", 2));
    }

    #[test]
    fn vertical_motion_remembers_column_across_short_lines() {
        let mut buf = Buffer::new("héllo wörld\nhi\nabcdefghij");
        buf.set_cursor("héllo w".len());
        buf.move_down();
        assert_eq!(buf.cursor(), "héllo wörld\nhi".len());
        buf.move_down();
        assert_eq!(buf.cursor(), "héllo wörld\nhi\nabcdefg".len());
        buf.move_up();
        buf.move_up();
        assert_eq!(buf.cursor(), "héllo w".len());
        buf.move_line_end();
        assert_eq!(buf.cursor(), "héllo wörld".len());
        buf.move_line_start();
        assert_eq!(buf.cursor(), 0);
    }

    #[test]
    fn set_cursor_rounds_down_to_char_boundary() {
        let mut buf = Buffer::new("aé");
        buf.set_cursor(2);
        assert_eq!(buf.cursor(), 1);
    }
}
