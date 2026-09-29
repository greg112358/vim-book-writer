use std::ops::Range;

/// Text with a cursor. The cursor is a byte offset that always sits on a char boundary.
/// Every edit goes through `replace`, which records it for undo.
#[derive(Debug, Default)]
pub struct Buffer {
    text: String,
    cursor: usize,
    /// Column (in chars) that up/down aim for, so a short line doesn't lose the column.
    goal_col: Option<usize>,
    undo: Vec<Vec<Edit>>,
    redo: Vec<Vec<Edit>>,
    /// Open `begin_group` calls. While above zero, edits join one undo step.
    group_depth: u32,
    /// True right after `begin_group` opens a step, before its first edit.
    group_fresh: bool,
    /// Counts every change to the text, undo and redo included.
    revision: u64,
    /// The revision when the text was last saved.
    saved_revision: u64,
}

/// One splice, stored so it can be reversed: `at..at + new_len` now holds what `old` replaced.
#[derive(Debug)]
struct Edit {
    at: usize,
    old: String,
    new_len: usize,
    cursor_before: usize,
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

    /// A number that differs whenever the text was changed since it was last read.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// True if the text was changed since `mark_saved`. Undoing back to the saved text still
    /// counts as a change.
    pub fn is_modified(&self) -> bool {
        self.revision != self.saved_revision
    }

    /// Where `pos` ends up after the edits of the last undo step, so a caller can keep a cursor
    /// in place around a change it made: text inserted at or before `pos` pushes it along.
    pub fn shift_through_last_step(&self, mut pos: usize) -> usize {
        for edit in self.undo.last().into_iter().flatten() {
            if pos >= edit.at + edit.old.len() {
                pos = pos - edit.old.len() + edit.new_len;
            } else if pos > edit.at {
                pos = edit.at + edit.new_len;
            }
        }
        pos
    }

    /// Records that the text as it is now was saved.
    pub fn mark_saved(&mut self) {
        self.saved_revision = self.revision;
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
        let edit = Edit {
            at: range.start,
            old: self.text[range.clone()].to_string(),
            new_len: with.len(),
            cursor_before: self.cursor,
        };
        match self.undo.last_mut() {
            Some(step) if self.group_depth > 0 && !self.group_fresh => step.push(edit),
            _ => self.undo.push(vec![edit]),
        }
        self.group_fresh = false;
        self.revision += 1;
        self.redo.clear();
        self.cursor = range.start + with.len();
        self.text.replace_range(range, with);
        self.goal_col = None;
    }

    /// Makes every edit until the matching `end_group` a single undo step. Calls nest.
    pub fn begin_group(&mut self) {
        if self.group_depth == 0 {
            self.group_fresh = true;
        }
        self.group_depth += 1;
    }

    pub fn end_group(&mut self) {
        self.group_depth = self.group_depth.saturating_sub(1);
    }

    /// Reverses the last undo step and puts the cursor where it was before it. False if none.
    pub fn undo(&mut self) -> bool {
        let Some(step) = self.undo.pop() else {
            return false;
        };
        let (inverse, cursor) = self.apply(step);
        self.redo.push(inverse);
        self.set_cursor(cursor);
        true
    }

    /// Re-applies the last undone step. False if none.
    pub fn redo(&mut self) -> bool {
        let Some(step) = self.redo.pop() else {
            return false;
        };
        let (inverse, _) = self.apply(step);
        self.undo.push(inverse);
        true
    }

    /// Reverses each edit of `step`, last first. Returns the edits that undo it again, and the
    /// cursor from before the step's first edit.
    fn apply(&mut self, step: Vec<Edit>) -> (Vec<Edit>, usize) {
        self.revision += 1;
        let mut inverse = Vec::with_capacity(step.len());
        let mut cursor = self.cursor;
        for edit in step.into_iter().rev() {
            let range = edit.at..edit.at + edit.new_len;
            inverse.push(Edit {
                at: edit.at,
                old: self.text[range.clone()].to_string(),
                new_len: edit.old.len(),
                cursor_before: self.cursor,
            });
            self.cursor = edit.at + edit.old.len();
            self.text.replace_range(range, &edit.old);
            cursor = edit.cursor_before;
        }
        self.goal_col = None;
        (inverse, cursor)
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

    /// Steps the cursor back onto the last char if it sits past the end of a non-empty line, as
    /// normal mode requires. Unlike `set_cursor`, it keeps the goal column.
    pub fn clamp_to_char(&mut self) {
        if self.cursor > self.line_start() && self.cursor == self.line_end() {
            self.cursor -= self.text[..self.cursor]
                .chars()
                .next_back()
                .map_or(0, char::len_utf8);
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
    fn undo_reverses_insert_and_delete_and_restores_cursor() {
        let mut buf = Buffer::new("héllo");
        buf.set_cursor(3);
        buf.insert("XY");
        buf.delete_back();
        assert_eq!((buf.text(), buf.cursor()), ("héXllo", 4));
        assert!(buf.undo());
        assert_eq!((buf.text(), buf.cursor()), ("héXYllo", 5));
        assert!(buf.undo());
        assert_eq!((buf.text(), buf.cursor()), ("héllo", 3));
        assert!(!buf.undo());
    }

    #[test]
    fn redo_replays_steps_and_new_edit_clears_it() {
        let mut buf = Buffer::default();
        for word in ["one", " two", " three"] {
            buf.insert(word);
        }
        while buf.undo() {}
        assert_eq!(buf.text(), "");
        assert!(buf.redo());
        assert!(buf.redo());
        assert_eq!((buf.text(), buf.cursor()), ("one two", 7));
        assert!(buf.undo());
        assert_eq!(buf.text(), "one");
        buf.insert("!");
        assert!(!buf.redo());
        assert_eq!(buf.text(), "one!");
    }

    #[test]
    fn group_is_one_undo_step_and_redoes_whole() {
        let mut buf = Buffer::new("ab");
        buf.set_cursor(1);
        buf.begin_group();
        buf.insert("1");
        buf.begin_group();
        buf.insert("2");
        buf.end_group();
        buf.replace(0..1, "AA");
        buf.end_group();
        buf.insert("!");
        assert_eq!(buf.text(), "AA!12b");
        assert!(buf.undo());
        assert!(buf.undo());
        assert_eq!((buf.text(), buf.cursor()), ("ab", 1));
        assert!(buf.redo());
        assert_eq!((buf.text(), buf.cursor()), ("AA12b", 2));
    }

    #[test]
    fn tracks_whether_the_text_changed_since_it_was_saved() {
        let mut buf = Buffer::new("a");
        assert!(!buf.is_modified());
        buf.insert("b");
        assert!(buf.is_modified());
        buf.mark_saved();
        assert!(!buf.is_modified());
        buf.undo();
        assert!(buf.is_modified());
        buf.mark_saved();
        buf.set_cursor(0);
        buf.move_right();
        assert!(!buf.is_modified());
    }

    #[test]
    fn set_cursor_rounds_down_to_char_boundary() {
        let mut buf = Buffer::new("aé");
        buf.set_cursor(2);
        assert_eq!(buf.cursor(), 1);
    }
}
