use crate::buffer::Buffer;
use std::ops::Range;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Ctrl(char),
    Esc,
    Enter,
    Backspace,
    Delete,
    Left,
    Right,
    Up,
    Down,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Normal,
    Insert,
    Visual,
    VisualLine,
}

/// Vim-style modal editing over a `Buffer`. Feed it keys with `handle`.
#[derive(Debug, Default)]
pub struct Vim {
    pub mode: Mode,
    /// Where the visual selection started; the buffer cursor is the other end.
    anchor: usize,
}

impl Vim {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn handle(&mut self, buf: &mut Buffer, key: Key) {
        match self.mode {
            Mode::Insert => match key {
                Key::Esc => {
                    buf.end_group();
                    self.mode = Mode::Normal;
                    if buf.cursor() > buf.line_start() {
                        buf.move_left();
                    }
                }
                Key::Char(c) => buf.insert(c.encode_utf8(&mut [0; 4])),
                Key::Enter => buf.insert("\n"),
                Key::Backspace => buf.delete_back(),
                Key::Delete => buf.delete_forward(),
                Key::Left => buf.move_left(),
                Key::Right => buf.move_right(),
                Key::Up => buf.move_up(),
                Key::Down => buf.move_down(),
                Key::Ctrl(_) => {}
            },
            Mode::Normal => {
                // An insert session, `o` and `O` included, is one undo step.
                match key {
                    Key::Char('i') => self.start_insert(buf),
                    Key::Char('a') => {
                        self.start_insert(buf);
                        if buf.cursor() < buf.line_end() {
                            buf.move_right();
                        }
                    }
                    Key::Char('A') => {
                        self.start_insert(buf);
                        buf.move_line_end();
                    }
                    Key::Char('I') => {
                        self.start_insert(buf);
                        let start = buf.line_start();
                        let blanks = buf.text()[start..]
                            .bytes()
                            .take_while(|b| matches!(b, b' ' | b'\t'))
                            .count();
                        buf.set_cursor(start + blanks);
                    }
                    Key::Char('o') => {
                        self.start_insert(buf);
                        buf.move_line_end();
                        buf.insert("\n");
                    }
                    Key::Char('O') => {
                        self.start_insert(buf);
                        buf.move_line_start();
                        buf.insert("\n");
                        buf.move_left();
                    }
                    Key::Char('v') => {
                        self.anchor = buf.cursor();
                        self.mode = Mode::Visual;
                    }
                    Key::Char('V') => {
                        self.anchor = buf.cursor();
                        self.mode = Mode::VisualLine;
                    }
                    _ => {}
                }
            }
            Mode::Visual | Mode::VisualLine => match key {
                Key::Esc => self.mode = Mode::Normal,
                Key::Char('v') => {
                    self.mode = if self.mode == Mode::Visual {
                        Mode::Normal
                    } else {
                        Mode::Visual
                    };
                }
                Key::Char('V') => {
                    self.mode = if self.mode == Mode::VisualLine {
                        Mode::Normal
                    } else {
                        Mode::VisualLine
                    };
                }
                _ => {}
            },
        }
    }

    fn start_insert(&mut self, buf: &mut Buffer) {
        buf.begin_group();
        self.mode = Mode::Insert;
    }

    /// The selected bytes in visual mode: both ends inclusive, or whole lines in visual-line mode.
    pub fn selection(&self, buf: &Buffer) -> Option<Range<usize>> {
        let text = buf.text();
        let (lo, hi) = (self.anchor.min(buf.cursor()), self.anchor.max(buf.cursor()));
        match self.mode {
            Mode::Visual => Some(lo..hi + text[hi..].chars().next().map_or(0, char::len_utf8)),
            Mode::VisualLine => {
                let start = text[..lo].rfind('\n').map_or(0, |i| i + 1);
                let end = text[hi..].find('\n').map_or(text.len(), |i| hi + i + 1);
                Some(start..end)
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feeds `keys` to the engine. `\x1b` is Esc, `\n` is Enter, `\x08` is Backspace.
    fn press(vim: &mut Vim, buf: &mut Buffer, keys: &str) {
        for c in keys.chars() {
            vim.handle(
                buf,
                match c {
                    '\x1b' => Key::Esc,
                    '\n' => Key::Enter,
                    '\x08' => Key::Backspace,
                    c => Key::Char(c),
                },
            );
        }
    }

    /// Runs `keys` from the start of `text`, or from `cursor` if given.
    fn run(text: &str, cursor: usize, keys: &str) -> (Vim, Buffer) {
        let (mut vim, mut buf) = (Vim::new(), Buffer::new(text));
        buf.set_cursor(cursor);
        press(&mut vim, &mut buf, keys);
        (vim, buf)
    }

    #[test]
    fn insert_commands_start_where_vim_puts_the_cursor() {
        // (text, cursor, keys, expected text)
        for (text, cursor, keys, want) in [
            ("ab", 0, "iX\x1b", "Xab"),
            ("ab", 0, "aX\x1b", "aXb"),
            ("ab", 1, "aX\x1b", "abX"),
            ("", 0, "aX\x1b", "X"),
            ("ab\ncd", 0, "AX\x1b", "abX\ncd"),
            ("  ab", 3, "IX\x1b", "  Xab"),
            ("a\nb", 0, "ox\x1b", "a\nx\nb"),
            ("a\nb", 2, "ox\x1b", "a\nb\nx"),
            ("a\nb", 2, "Ox\x1b", "a\nx\nb"),
            ("a", 0, "Ox\x1b", "x\na"),
            ("a", 0, "ix\ny\x08\x08z\x1b", "xza"),
        ] {
            let (vim, buf) = run(text, cursor, keys);
            assert_eq!(buf.text(), want, "{keys:?} on {text:?}");
            assert_eq!(vim.mode, Mode::Normal);
        }
    }

    #[test]
    fn escape_steps_the_cursor_back_except_at_line_start() {
        let (_, buf) = run("abc", 0, "iXY\x1b");
        assert_eq!(buf.cursor(), 1);
        let (_, buf) = run("abc", 0, "i\x1b");
        assert_eq!(buf.cursor(), 0);
        let (_, buf) = run("a\nb", 0, "o\x1b");
        assert_eq!(buf.cursor(), 2);
    }

    #[test]
    fn an_insert_session_is_one_undo_step() {
        for keys in ["ihello\n world\x1b", "oone\ntwo\x1b", "Oone\ntwo\x1b"] {
            let (_, mut buf) = run("a", 0, keys);
            assert!(buf.undo());
            assert_eq!(buf.text(), "a", "{keys:?}");
            assert!(!buf.undo());
        }
    }

    #[test]
    fn visual_modes_switch_and_leave() {
        let (mut vim, mut buf) = run("abc", 1, "v");
        assert_eq!(vim.mode, Mode::Visual);
        assert_eq!(vim.selection(&buf), Some(1..2));
        press(&mut vim, &mut buf, "V");
        assert_eq!(vim.mode, Mode::VisualLine);
        press(&mut vim, &mut buf, "v");
        assert_eq!(vim.mode, Mode::Visual);
        press(&mut vim, &mut buf, "v");
        assert_eq!(vim.mode, Mode::Normal);
        assert_eq!(vim.selection(&buf), None);
        press(&mut vim, &mut buf, "V");
        press(&mut vim, &mut buf, "V");
        assert_eq!(vim.mode, Mode::Normal);
        press(&mut vim, &mut buf, "v\x1b");
        assert_eq!(vim.mode, Mode::Normal);
        press(&mut vim, &mut buf, "V\x1b");
        assert_eq!(vim.mode, Mode::Normal);
        // Keys that start an insert don't in visual mode.
        press(&mut vim, &mut buf, "viX\x1b");
        assert_eq!(buf.text(), "abc");
    }

    #[test]
    fn selection_covers_whole_chars_and_whole_lines() {
        let (vim, buf) = run("aé\nbc\nd", 1, "v");
        assert_eq!(vim.selection(&buf), Some(1..3));
        let (vim, buf) = run("ab\ncd\nef", 4, "V");
        assert_eq!(vim.selection(&buf), Some(3..6));
        let (vim, buf) = run("ab\ncd", 4, "V");
        assert_eq!(vim.selection(&buf), Some(3..5));
    }
}
