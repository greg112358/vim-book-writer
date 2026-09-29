use crate::buffer::Buffer;
use crate::motion::{first_non_blank, nth_line_start, word_back, word_end, word_forward};
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

#[derive(Debug, Default)]
struct Pending {
    count: Option<usize>,
    /// A `g` was typed and the next key completes it.
    g: bool,
}

/// Vim-style modal editing over a `Buffer`. Feed it keys with `handle`.
#[derive(Debug, Default)]
pub struct Vim {
    pub mode: Mode,
    /// Where the visual selection started; the buffer cursor is the other end.
    anchor: usize,
    pending: Pending,
}

impl Vim {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn handle(&mut self, buf: &mut Buffer, key: Key) {
        if self.mode == Mode::Insert {
            match key {
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
            }
            return;
        }

        let Pending { count, g } = std::mem::take(&mut self.pending);
        let visual = self.mode != Mode::Normal;
        let n = count.unwrap_or(1);
        match key {
            Key::Esc => {
                if visual {
                    self.mode = Mode::Normal;
                }
            }
            Key::Char('g') if !g => self.pending = Pending { count, g: true },
            Key::Char('g') => {
                let text = buf.text();
                buf.set_cursor(first_non_blank(text, nth_line_start(text, n - 1)));
            }
            // Any other key after `g` isn't a command we have.
            _ if g => {}
            Key::Char(d @ '0'..='9') if d != '0' || count.is_some() => {
                let digit = d as usize - '0' as usize;
                self.pending.count =
                    Some(count.unwrap_or(0).saturating_mul(10).saturating_add(digit));
            }
            // An insert session, `o` and `O` included, is one undo step.
            Key::Char(c @ ('i' | 'a' | 'A' | 'I' | 'o' | 'O')) if !visual => {
                buf.begin_group();
                self.mode = Mode::Insert;
                match c {
                    'a' if buf.cursor() < buf.line_end() => buf.move_right(),
                    'A' => buf.move_line_end(),
                    'I' => buf.set_cursor(first_non_blank(buf.text(), buf.cursor())),
                    'o' => {
                        buf.move_line_end();
                        buf.insert("\n");
                    }
                    'O' => {
                        buf.move_line_start();
                        buf.insert("\n");
                        buf.move_left();
                    }
                    _ => {}
                }
            }
            Key::Char('v') => {
                if self.mode == Mode::Normal {
                    self.anchor = buf.cursor();
                }
                self.mode = if self.mode == Mode::Visual {
                    Mode::Normal
                } else {
                    Mode::Visual
                };
            }
            Key::Char('V') => {
                if self.mode == Mode::Normal {
                    self.anchor = buf.cursor();
                }
                self.mode = if self.mode == Mode::VisualLine {
                    Mode::Normal
                } else {
                    Mode::VisualLine
                };
            }
            _ => move_cursor(buf, key, count),
        }
        // Normal and visual mode never rest past the last char of a line.
        if self.mode != Mode::Insert {
            buf.clamp_to_char();
        }
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

/// Moves the cursor for a motion key, `count` times. Other keys do nothing. Motions stop early
/// when they can't go further, so a huge count is cheap.
fn move_cursor(buf: &mut Buffer, key: Key, count: Option<usize>) {
    let n = count.unwrap_or(1);
    match key {
        Key::Char('h') | Key::Left => {
            for _ in 0..n {
                if buf.cursor() == buf.line_start() {
                    break;
                }
                buf.move_left();
            }
        }
        Key::Char('l') | Key::Right => {
            for _ in 0..n {
                let at = buf.cursor();
                match buf.text()[at..].chars().next() {
                    Some(c) if at + c.len_utf8() < buf.line_end() => buf.move_right(),
                    _ => break,
                }
            }
        }
        // The last line, or the one before a final newline, is where `j` stops.
        Key::Char('j') | Key::Down => {
            for _ in 0..n {
                if buf.line_end() + 1 >= buf.text().len() {
                    break;
                }
                buf.move_down();
            }
        }
        Key::Char('k') | Key::Up => {
            for _ in 0..n {
                if buf.line_start() == 0 {
                    break;
                }
                buf.move_up();
            }
        }
        Key::Char(c @ ('w' | 'b' | 'e')) => {
            let step = match c {
                'w' => word_forward,
                'b' => word_back,
                _ => word_end,
            };
            for _ in 0..n {
                let at = buf.cursor();
                let to = step(buf.text(), at);
                if to == at {
                    break;
                }
                buf.set_cursor(to);
            }
        }
        Key::Char('0') => buf.move_line_start(),
        Key::Char('^') => buf.set_cursor(first_non_blank(buf.text(), buf.cursor())),
        Key::Char('$') => {
            for _ in 1..n {
                if buf.line_end() + 1 >= buf.text().len() {
                    break;
                }
                buf.move_down();
            }
            buf.move_line_end();
        }
        Key::Char('G') => {
            let text = buf.text();
            let line = count.map_or(usize::MAX, |c| c - 1);
            buf.set_cursor(first_non_blank(text, nth_line_start(text, line)));
        }
        _ => {}
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

    #[test]
    fn horizontal_motions_stay_on_the_line() {
        // (text, start, keys, cursor after)
        for (text, start, keys, want) in [
            (
                "abc
def", 0, "l", 1,
            ),
            (
                "abc
def", 0, "5l", 2,
            ),
            (
                "abc
def", 2, "l", 2,
            ),
            (
                "abc
def", 4, "h", 4,
            ),
            (
                "abc
def", 6, "10h", 4,
            ),
            (
                "aé
x", 0, "l", 1,
            ),
            ("  ab cd", 3, "0", 0),
            ("  ab cd", 6, "^", 2),
            ("  ab cd", 0, "$", 6),
            (
                "aé
x", 0, "$", 1,
            ),
            (
                "ab

cd", 3, "$", 3,
            ),
            (
                "ab
cd
ef", 0, "2$", 4,
            ),
            (
                "ab
cd", 0, "9$", 4,
            ),
            ("0123456789abc", 0, "10l", 10),
        ] {
            let (_, buf) = run(text, start, keys);
            assert_eq!(buf.cursor(), want, "{keys:?} from {start} in {text:?}");
        }
    }

    #[test]
    fn word_motions_with_counts() {
        // Words: one two three, an empty line at 15, four five.
        let text = "one two  three\n\nfour five";
        for (start, keys, want) in [
            (0, "w", 4),
            (0, "2w", 9),
            (0, "3w", 15),
            (0, "4w", 16),
            (0, "5w", 21),
            (0, "99w", 24),
            (4, "b", 0),
            (24, "3b", 15),
            (0, "e", 2),
            (0, "2e", 6),
            (0, "3e", 13),
            (2, "e", 6),
        ] {
            let (_, buf) = run(text, start, keys);
            assert_eq!(buf.cursor(), want, "{keys:?} from {start}");
        }
    }

    #[test]
    fn vertical_motions_keep_the_column_and_stop_at_the_last_line() {
        let text = "abcdef\nab\nabcdef\n";
        let (mut vim, mut buf) = run(text, 4, "j");
        assert_eq!(buf.cursor(), 8);
        press(&mut vim, &mut buf, "j");
        assert_eq!(buf.cursor(), 14);
        // The final newline ends the last line; there is no line after it.
        press(&mut vim, &mut buf, "j5j");
        assert_eq!(buf.cursor(), 14);
        press(&mut vim, &mut buf, "kk");
        assert_eq!(buf.cursor(), 4);
        press(&mut vim, &mut buf, "k");
        assert_eq!(buf.cursor(), 4);
        press(&mut vim, &mut buf, "2j");
        assert_eq!(buf.cursor(), 14);
        let (_, buf) = run("a\nb", 0, "9j");
        assert_eq!(buf.cursor(), 2);
    }

    #[test]
    fn line_jumps_land_on_the_first_non_blank() {
        // Lines start at 0, 4, and 7; their first non-blanks are 2, 5, and 7.
        let text = "  a\n b\nc\n";
        for (start, keys, want) in [
            (0, "G", 7),
            (0, "2G", 5),
            (0, "99G", 7),
            (7, "gg", 2),
            (7, "2gg", 5),
            (0, "1G", 2),
        ] {
            let (_, buf) = run(text, start, keys);
            assert_eq!(buf.cursor(), want, "{keys:?} from {start}");
        }
    }

    #[test]
    fn pending_count_and_g_are_dropped_by_esc_or_a_stray_key() {
        let text = "a b c d e";
        for (keys, want) in [
            ("3\x1bw", 2),
            ("gxw", 2),
            ("3gxw", 2),
            ("2w3w", 8),
            ("g\x1bw", 2),
        ] {
            let (_, buf) = run(text, 0, keys);
            assert_eq!(buf.cursor(), want, "{keys:?}");
        }
    }

    #[test]
    fn motions_extend_the_visual_selection() {
        let (vim, buf) = run("one two three", 0, "vw");
        assert_eq!(vim.selection(&buf), Some(0..5));
        let (vim, buf) = run("ab\ncd\nef", 0, "Vj");
        assert_eq!(vim.selection(&buf), Some(0..6));
        let (vim, buf) = run("one two", 6, "vbb");
        assert_eq!(vim.selection(&buf), Some(0..7));
    }
}
