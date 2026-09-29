use crate::buffer::Buffer;
use crate::motion::{
    first_non_blank, line_end, line_start, nth_line_start, word_back, word_end, word_forward,
    word_run_end,
};
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

#[derive(Debug, Clone)]
struct Register {
    text: String,
    /// Whole lines, each ending in `\n`. `p` puts them on their own lines.
    linewise: bool,
}

#[derive(Debug, Default)]
struct Pending {
    /// Digits typed so far for the next thing: the operator, or its motion.
    count: Option<usize>,
    /// The count typed before the operator, the 2 in `2d3w`.
    before: Option<usize>,
    /// `d`, `c`, or `y`, waiting for its motion.
    op: Option<char>,
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
    unnamed: Option<Register>,
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

        let p = std::mem::take(&mut self.pending);
        let visual = self.mode != Mode::Normal;
        // The whole count: `2d3w` is 6.
        let count = match (p.before, p.count) {
            (None, None) => None,
            (a, b) => Some(a.unwrap_or(1).saturating_mul(b.unwrap_or(1))),
        };
        let n = count.unwrap_or(1);
        match key {
            Key::Esc => {
                if visual {
                    self.mode = Mode::Normal;
                }
            }
            Key::Char('g') if !p.g => self.pending = Pending { g: true, ..p },
            // Any other key after `g` isn't a command we have.
            _ if p.g && key != Key::Char('g') => {}
            Key::Char(d @ '0'..='9') if d != '0' || p.count.is_some() => {
                let digit = d as usize - '0' as usize;
                let count = p
                    .count
                    .unwrap_or(0)
                    .saturating_mul(10)
                    .saturating_add(digit);
                self.pending = Pending {
                    count: Some(count),
                    ..p
                };
            }
            Key::Char(op @ ('d' | 'c' | 'y')) if visual => self.visual_operator(buf, op),
            Key::Char('x') if visual => self.visual_operator(buf, 'd'),
            // Capital operators work on whole lines.
            Key::Char(c @ ('X' | 'D' | 'Y' | 'C' | 'S')) if visual => {
                self.mode = Mode::VisualLine;
                let op = match c {
                    'X' | 'D' => 'd',
                    'Y' => 'y',
                    _ => 'c',
                };
                self.visual_operator(buf, op);
            }
            Key::Char(c @ ('d' | 'c' | 'y')) => match p.op {
                None => {
                    self.pending = Pending {
                        before: p.count,
                        op: Some(c),
                        ..Pending::default()
                    }
                }
                Some(op) if op == c => self.lines_operator(buf, op, count),
                Some(_) => {}
            },
            _ if p.op.is_some() => self.operator(buf, p.op.unwrap(), key, p.g, count),
            Key::Char('x') => self.operator(buf, 'd', Key::Char('l'), false, count),
            Key::Char('X') => self.operator(buf, 'd', Key::Char('h'), false, count),
            Key::Char('D') => self.operator(buf, 'd', Key::Char('$'), false, count),
            Key::Char('C') => self.operator(buf, 'c', Key::Char('$'), false, count),
            Key::Char('s') => self.operator(buf, 'c', Key::Char('l'), false, count),
            Key::Char('S') => self.lines_operator(buf, 'c', count),
            Key::Char('Y') => self.lines_operator(buf, 'y', count),
            Key::Char(c @ ('p' | 'P')) if !visual => self.paste(buf, c == 'P', n),
            Key::Char('u') if !visual => {
                for _ in 0..n {
                    if !buf.undo() {
                        break;
                    }
                }
            }
            Key::Ctrl('r') if !visual => {
                for _ in 0..n {
                    if !buf.redo() {
                        break;
                    }
                }
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
            _ => move_cursor(buf, key, p.g, count),
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

    fn visual_operator(&mut self, buf: &mut Buffer, op: char) {
        let range = self.selection(buf).unwrap_or_default();
        let linewise = self.mode == Mode::VisualLine;
        self.mode = Mode::Normal;
        self.apply_operator(buf, op, range.clone(), linewise, range.start);
    }

    /// `d`, `c`, or `y` from the cursor over the motion `key` (`g` set means `gg`).
    fn operator(&mut self, buf: &mut Buffer, op: char, key: Key, g: bool, count: Option<usize>) {
        if !is_motion(key, g) {
            return;
        }
        let n = count.unwrap_or(1);
        let start = buf.cursor();
        let text = buf.text();
        let linewise = g || matches!(key, Key::Char('j' | 'k' | 'G') | Key::Up | Key::Down);
        let mut inclusive = matches!(key, Key::Char('e' | '$'));
        let target = match key {
            // Unlike plain `l`, this may reach the end of the line, so `x` can take the last char.
            Key::Char('l') | Key::Right if !g => {
                let mut to = start;
                for _ in 0..n {
                    match text[to..].chars().next() {
                        Some(c) if c != '\n' => to += c.len_utf8(),
                        _ => break,
                    }
                }
                to
            }
            // `cw` on a word changes to the end of the word, like `ce` but without moving on
            // when already at its last char.
            Key::Char('w')
                if op == 'c'
                    && text[start..]
                        .chars()
                        .next()
                        .is_some_and(|c| !c.is_whitespace()) =>
            {
                inclusive = true;
                let mut to = word_run_end(text, start);
                for _ in 1..n {
                    let next = word_end(text, to);
                    if next == to {
                        break;
                    }
                    to = next;
                }
                to
            }
            // `dw` on the last word of a line stops at the line's end instead of joining the
            // next line.
            Key::Char('w') if !g => {
                let (mut prev, mut to) = (start, start);
                for _ in 0..n {
                    let next = word_forward(text, to);
                    if next == to {
                        break;
                    }
                    (prev, to) = (to, next);
                }
                if text[prev..to].contains('\n') {
                    to = line_end(text, prev);
                }
                to
            }
            _ => {
                move_cursor(buf, key, g, count);
                let to = buf.cursor();
                buf.set_cursor(start);
                to
            }
        };
        if !g && matches!(key, Key::Char('j' | 'k') | Key::Up | Key::Down) && target == start {
            return;
        }
        let text = buf.text();
        let (lo, hi) = (start.min(target), start.max(target));
        let range = if linewise {
            line_start(text, lo)..(line_end(text, hi) + 1).min(text.len())
        } else if inclusive {
            let last = text[hi..].chars().next().filter(|&c| c != '\n');
            lo..hi + last.map_or(0, char::len_utf8)
        } else {
            lo..hi
        };
        self.apply_operator(buf, op, range, linewise, lo);
    }

    /// The doubled forms `dd`, `cc`, and `yy`: `count` whole lines from the cursor's.
    fn lines_operator(&mut self, buf: &mut Buffer, op: char, count: Option<usize>) {
        let text = buf.text();
        let start = line_start(text, buf.cursor());
        let mut end = line_end(text, buf.cursor());
        for _ in 1..count.unwrap_or(1) {
            if end + 1 >= text.len() {
                break;
            }
            end = line_end(text, end + 1);
        }
        let range = start..(end + 1).min(text.len());
        self.apply_operator(buf, op, range, true, buf.cursor());
    }

    /// Yanks, deletes, or changes `range`. `cursor` is where a yank leaves the cursor.
    fn apply_operator(
        &mut self,
        buf: &mut Buffer,
        op: char,
        mut range: Range<usize>,
        linewise: bool,
        cursor: usize,
    ) {
        if range.is_empty() && op != 'c' {
            return;
        }
        let text = buf.text();
        let mut yanked = text[range.clone()].to_string();
        if linewise && !yanked.ends_with('\n') {
            yanked.push('\n');
        }
        self.unnamed = Some(Register {
            text: yanked,
            linewise,
        });
        let ends_with_newline = text[..range.end].ends_with('\n');
        match op {
            'y' => buf.set_cursor(cursor),
            'd' => {
                // The last line has no newline of its own, so take the one before it.
                if linewise && range.start > 0 && !ends_with_newline {
                    range.start -= 1;
                }
                let line = text[..range.start].matches('\n').count();
                buf.replace(range.clone(), "");
                let text = buf.text();
                buf.set_cursor(if linewise {
                    first_non_blank(text, nth_line_start(text, line))
                } else {
                    range.start
                });
            }
            _ => {
                buf.begin_group();
                self.mode = Mode::Insert;
                // Changing lines empties them but keeps the last line break.
                if linewise && ends_with_newline {
                    range.end -= 1;
                }
                if !range.is_empty() {
                    buf.replace(range.clone(), "");
                }
                buf.set_cursor(range.start);
            }
        }
    }

    /// `p` after the cursor or line, `P` before, `count` times.
    fn paste(&mut self, buf: &mut Buffer, before: bool, count: usize) {
        let Some(register) = &self.unnamed else {
            return;
        };
        let text = register.text.repeat(count);
        let cursor = buf.cursor();
        if register.linewise {
            let (at, insert) = if before {
                let at = buf.line_start();
                (at, text)
            } else if buf.line_end() < buf.text().len() {
                (buf.line_end() + 1, text)
            } else {
                // The last line has no newline to insert after, so bring one.
                let body = text.strip_suffix('\n').unwrap_or(&text);
                (buf.text().len(), format!("\n{body}"))
            };
            buf.replace(at..at, &insert);
            let first = at + usize::from(insert.starts_with('\n') && at > 0 && !before);
            buf.set_cursor(first_non_blank(buf.text(), first));
        } else {
            let under = buf.text()[cursor..].chars().next().filter(|&c| c != '\n');
            let at = if before {
                cursor
            } else {
                cursor + under.map_or(0, char::len_utf8)
            };
            buf.replace(at..at, &text);
            // Like vim: on the last char of a one-line paste, at the start of a longer one.
            let last = text.chars().next_back().map_or(0, char::len_utf8);
            buf.set_cursor(if text.contains('\n') {
                at
            } else {
                at + text.len() - last
            });
        }
    }
}

/// True for the keys that are cursor motions, or, with `g`, for the second `g` of `gg`.
fn is_motion(key: Key, g: bool) -> bool {
    if g {
        return key == Key::Char('g');
    }
    matches!(
        key,
        Key::Char('h' | 'j' | 'k' | 'l' | 'w' | 'b' | 'e' | '0' | '^' | '$' | 'G')
            | Key::Left
            | Key::Right
            | Key::Up
            | Key::Down
    )
}

/// Moves the cursor for a motion key, `count` times. Other keys do nothing. `g` set means the
/// key completes `gg`. Motions stop early when they can't go further, so a huge count is cheap.
fn move_cursor(buf: &mut Buffer, key: Key, g: bool, count: Option<usize>) {
    let n = count.unwrap_or(1);
    if g {
        let text = buf.text();
        buf.set_cursor(first_non_blank(text, nth_line_start(text, n - 1)));
        return;
    }
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
                    '\x12' => Key::Ctrl('r'),
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
        press(&mut vim, &mut buf, "viz\x1b");
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

    #[test]
    fn delete_and_change_with_motions() {
        // (text, cursor, keys, text after, cursor after)
        for (text, cursor, keys, want, at) in [
            ("one two three", 0, "dw", "two three", 0),
            ("one two three", 0, "d2w", "three", 0),
            ("one two three", 0, "2dw", "three", 0),
            ("one two three", 0, "2d2w", "", 0),
            ("a foo\nb", 2, "dw", "a \nb", 1),
            ("a foo  \nb", 2, "dw", "a \nb", 1),
            ("a b\nc d", 0, "d2w", "\nc d", 0),
            ("one two", 6, "db", "one o", 4),
            ("one two", 0, "de", " two", 0),
            ("one two", 4, "d0", "two", 0),
            ("  one two", 6, "d^", "  two", 2),
            ("one two", 4, "d$", "one ", 3),
            ("one two\nx", 4, "D", "one \nx", 3),
            ("ab\n\ncd", 3, "D", "ab\n\ncd", 3),
            ("aéb", 1, "x", "ab", 1),
            ("ab", 1, "x", "a", 0),
            ("abcd", 0, "3x", "d", 0),
            ("abcd", 1, "X", "bcd", 0),
            ("ab\ncd", 1, "3x", "a\ncd", 0),
            ("abcd", 3, "dh", "abd", 2),
            ("abcd", 0, "dh", "abcd", 0),
            ("abcd", 2, "dl", "abd", 2),
            ("one two", 0, "cwX\x1b", "X two", 0),
            ("ab cd", 1, "cwX\x1b", "aX cd", 1),
            ("one two three", 0, "c2wX\x1b", "X three", 0),
            ("a  b", 1, "cwX\x1b", "aXb", 1),
            ("foo.bar", 0, "cwX\x1b", "X.bar", 0),
            ("one two", 0, "ceX\x1b", "X two", 0),
            ("one two", 4, "CX\x1b", "one X", 4),
            ("abc", 1, "sX\x1b", "aXc", 1),
            ("abc", 1, "3sX\x1b", "aX", 1),
        ] {
            let (_, buf) = run(text, cursor, keys);
            assert_eq!(
                (buf.text(), buf.cursor()),
                (want, at),
                "{keys:?} on {text:?} at {cursor}"
            );
        }
    }

    #[test]
    fn linewise_operators() {
        for (text, cursor, keys, want, at) in [
            ("a\nb\nc", 2, "dd", "a\nc", 2),
            ("a\nb\nc", 4, "dd", "a\nb", 2),
            ("a", 0, "dd", "", 0),
            ("a\nb\nc\n", 4, "dd", "a\nb\n", 2),
            ("a\n  b\nc", 0, "dd", "  b\nc", 2),
            ("a\nb\nc\nd", 0, "2dd", "c\nd", 0),
            ("a\nb\nc\nd", 2, "9dd", "a", 0),
            ("a\nb\nc\nd", 2, "dj", "a\nd", 2),
            ("a\nb\nc\nd", 2, "dk", "c\nd", 0),
            ("a\nb\nc\nd", 2, "dG", "a", 0),
            ("a\nb\nc\nd", 4, "dgg", "d", 0),
            ("a\nb", 2, "dj", "a\nb", 2),
            ("a\nb", 0, "dk", "a\nb", 0),
            ("a\n  b\nc", 2, "ccX\x1b", "a\nX\nc", 2),
            ("a\nb\nc", 0, "2ccX\x1b", "X\nc", 0),
            ("a\nb\nc", 0, "cjX\x1b", "X\nc", 0),
            ("a\nb", 2, "SX\x1b", "a\nX", 2),
        ] {
            let (_, buf) = run(text, cursor, keys);
            assert_eq!(
                (buf.text(), buf.cursor()),
                (want, at),
                "{keys:?} on {text:?} at {cursor}"
            );
        }
    }

    #[test]
    fn yank_and_paste() {
        for (text, cursor, keys, want, at) in [
            ("a\nb", 0, "yyp", "a\na\nb", 2),
            ("a\nb", 2, "yyp", "a\nb\nb", 4),
            ("a\nb", 2, "yyP", "a\nb\nb", 2),
            ("a\nb", 0, "yyjp", "a\nb\na", 4),
            ("a\nb\n", 2, "yyp", "a\nb\nb\n", 4),
            ("a\n  b", 0, "jyyP", "a\n  b\n  b", 4),
            ("a\nb", 0, "3yyP", "a\nb\na\nb", 0),
            ("a\nb", 0, "yyj3p", "a\nb\na\na\na", 4),
            ("a\nb", 0, "Yjp", "a\nb\na", 4),
            ("a\nb", 0, "ddp", "b\na", 2),
            ("ab", 0, "xp", "ba", 1),
            ("ab", 0, "xP", "ab", 0),
            ("one two", 0, "ywP", "one one two", 3),
            ("one two", 0, "yw$p", "one twoone ", 10),
            ("one two", 4, "ybP", "one one two", 3),
            ("abc", 0, "yl3p", "aaaabc", 3),
            ("a b", 0, "dwp", "ba ", 2),
            ("one\ntwo", 2, "y$jp", "one\ntwoe", 7),
            ("ab\ncd", 1, "yjP", "ab\ncd\nab\ncd", 0),
            ("ab", 0, "p", "ab", 0),
        ] {
            let (_, buf) = run(text, cursor, keys);
            assert_eq!(
                (buf.text(), buf.cursor()),
                (want, at),
                "{keys:?} on {text:?} at {cursor}"
            );
        }
    }

    #[test]
    fn multi_line_charwise_paste_puts_the_cursor_at_its_start() {
        let (_, buf) = run("ab\ncd\nef", 1, "vjyGp");
        assert_eq!(buf.text(), "ab\ncd\neb\ncdf");
        assert_eq!(buf.cursor(), 7);
    }

    #[test]
    fn undo_and_redo() {
        let (mut vim, mut buf) = run("one two three", 0, "dwdw");
        assert_eq!(buf.text(), "three");
        press(&mut vim, &mut buf, "u");
        assert_eq!((buf.text(), buf.cursor()), ("two three", 0));
        press(&mut vim, &mut buf, "\x12");
        assert_eq!(buf.text(), "three");
        press(&mut vim, &mut buf, "2u");
        assert_eq!(buf.text(), "one two three");
        press(&mut vim, &mut buf, "9u");
        assert_eq!(buf.text(), "one two three");
        press(&mut vim, &mut buf, "2\x12");
        assert_eq!(buf.text(), "three");
        // A change is one step, however many edits: cw deletes and then inserts.
        let (mut vim, mut buf) = run("one two", 0, "cwfoo\x1b");
        assert_eq!(buf.text(), "foo two");
        press(&mut vim, &mut buf, "u");
        assert_eq!((buf.text(), buf.cursor()), ("one two", 0));
        // So is a linewise delete of the last line and an insert after o.
        let (mut vim, mut buf) = run("a\nb", 2, "dd");
        press(&mut vim, &mut buf, "u");
        assert_eq!(buf.text(), "a\nb");
    }

    #[test]
    fn operators_cancel_on_a_stray_key_and_keep_counts_apart() {
        for (keys, want) in [
            ("dzx", "bc"),
            ("d\x1bx", "bc"),
            ("dcx", "bc"),
            ("dgx", "abc"),
            ("2d\x1bx", "bc"),
            ("d3\x1bx", "bc"),
        ] {
            let (_, buf) = run("abc", 0, keys);
            assert_eq!(buf.text(), want, "{keys:?}");
        }
    }

    #[test]
    fn visual_operators() {
        for (text, cursor, keys, want) in [
            ("one two", 0, "vwd", "wo"),
            ("one two", 0, "vex", " two"),
            ("a\nb\nc", 2, "Vd", "a\nc"),
            ("a\nb\nc", 0, "Vjd", "c"),
            ("one two", 0, "vey$p", "one twoone"),
            ("a\nb", 0, "Vyjp", "a\nb\na"),
            ("one two", 0, "vecX\x1b", "X two"),
            ("a\nb\nc", 2, "VcX\x1b", "a\nX\nc"),
            ("a\nb\nc", 2, "vX", "a\nc"),
            ("a\nb\nc", 0, "vjD", "c"),
            ("a\nb", 0, "vYjp", "a\nb\na"),
            ("a\nb\nc", 2, "vCX\x1b", "a\nX\nc"),
        ] {
            let (vim, buf) = run(text, cursor, keys);
            assert_eq!(buf.text(), want, "{keys:?} on {text:?}");
            assert_eq!(vim.mode, Mode::Normal);
        }
        let (mut vim, mut buf) = run("one two", 0, "vwd");
        press(&mut vim, &mut buf, "u");
        assert_eq!(buf.text(), "one two");
        // p and u do nothing in visual mode yet, and don't end it.
        let (vim, buf) = run("abc", 0, "vlpu");
        assert_eq!((buf.text(), vim.mode), ("abc", Mode::Visual));
    }
}
