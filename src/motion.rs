//! Cursor motions and text objects as pure functions on text and a byte position.

use std::ops::Range;

/// 0 for whitespace, 1 for word chars, 2 for punctuation. `w`, `b`, and `e` stop where it changes.
fn class(c: char) -> u8 {
    class_by(c, false)
}

/// Like `class`, but with `big` (for `W`, `B`, `E`) every non-blank char is one class.
fn class_by(c: char, big: bool) -> u8 {
    if c.is_whitespace() {
        0
    } else if big || c.is_alphanumeric() || c == '_' {
        1
    } else {
        2
    }
}

pub fn line_start(text: &str, pos: usize) -> usize {
    text[..pos].rfind('\n').map_or(0, |i| i + 1)
}

/// The position of the line's `\n`, or the end of the text.
pub fn line_end(text: &str, pos: usize) -> usize {
    text[pos..].find('\n').map_or(text.len(), |i| pos + i)
}

/// The first char of the line that isn't a space or tab, or the line's end if there is none.
pub fn first_non_blank(text: &str, pos: usize) -> usize {
    let start = line_start(text, pos);
    start
        + text[start..]
            .bytes()
            .take_while(|b| matches!(b, b' ' | b'\t'))
            .count()
}

/// Start of line `n`, counting from 0. Past the last line it is the last line, and a final
/// `\n` ends a line rather than starting another.
pub fn nth_line_start(text: &str, n: usize) -> usize {
    let mut pos = 0;
    for _ in 0..n {
        match text[pos..].find('\n') {
            Some(i) if pos + i + 1 < text.len() => pos += i + 1,
            _ => break,
        }
    }
    pos
}

/// `w`: the start of the next word. An empty line counts as a word.
pub fn word_forward(text: &str, pos: usize) -> usize {
    word_forward_by(text, pos, false)
}

pub fn word_forward_by(text: &str, pos: usize, big: bool) -> usize {
    let mut chars = text[pos..].char_indices().peekable();
    let Some(&(_, first)) = chars.peek() else {
        return text.len();
    };
    let start_class = class_by(first, big);
    if start_class != 0 {
        while chars
            .next_if(|&(_, c)| class_by(c, big) == start_class)
            .is_some()
        {}
    }
    for (offset, c) in chars {
        let i = pos + offset;
        if !c.is_whitespace() {
            return i;
        }
        if c == '\n' && i > pos && (i == 0 || text.as_bytes()[i - 1] == b'\n') {
            return i;
        }
    }
    text.len()
}

/// `b`: the start of this word if inside one, else of the previous word. An empty line counts
/// as a word.
pub fn word_back(text: &str, pos: usize) -> usize {
    word_back_by(text, pos, false)
}

pub fn word_back_by(text: &str, pos: usize, big: bool) -> usize {
    let mut i = pos;
    let word_class = loop {
        let Some(c) = text[..i].chars().next_back() else {
            return 0;
        };
        if !c.is_whitespace() {
            break class_by(c, big);
        }
        i -= c.len_utf8();
        if c == '\n' && (i == 0 || text.as_bytes()[i - 1] == b'\n') {
            return i;
        }
    };
    while let Some(c) = text[..i]
        .chars()
        .next_back()
        .filter(|&c| class_by(c, big) == word_class)
    {
        i -= c.len_utf8();
    }
    i
}

/// The last char of the run of same-class chars that holds `pos`.
pub fn word_run_end(text: &str, pos: usize) -> usize {
    word_run_end_by(text, pos, false)
}

pub fn word_run_end_by(text: &str, pos: usize, big: bool) -> usize {
    let Some(first) = text[pos..].chars().next() else {
        return pos;
    };
    let run_class = class_by(first, big);
    let mut end = pos;
    for (offset, c) in text[pos..].char_indices() {
        if class_by(c, big) != run_class {
            break;
        }
        end = pos + offset;
    }
    end
}

/// `e`: the last char of this word, or if already there, of the next one. Stays put if there
/// is no next word.
pub fn word_end(text: &str, pos: usize) -> usize {
    word_end_by(text, pos, false)
}

pub fn word_end_by(text: &str, pos: usize, big: bool) -> usize {
    let mut chars = text[pos..]
        .char_indices()
        .skip(1)
        .skip_while(|&(_, c)| c.is_whitespace());
    let Some((offset, first)) = chars.next() else {
        return pos;
    };
    let word_class = class_by(first, big);
    let mut end = pos + offset;
    for (offset, c) in chars {
        if class_by(c, big) != word_class {
            break;
        }
        end = pos + offset;
    }
    end
}

/// True if the line starting at `start` is empty or only blanks.
fn blank_line(text: &str, start: usize) -> bool {
    text[start..line_end(text, start)].trim().is_empty()
}

/// `}`: the start of the next blank line after the paragraph the cursor is in, or the end of the
/// text if there is none.
pub fn paragraph_forward(text: &str, pos: usize) -> usize {
    let mut at = line_start(text, pos);
    for want_blank in [false, true] {
        while blank_line(text, at) != want_blank {
            let end = line_end(text, at);
            if end == text.len() {
                return text.len();
            }
            at = end + 1;
        }
    }
    at
}

/// `{`: the start of the blank line before the paragraph the cursor is in, or the start of the
/// text if there is none.
pub fn paragraph_back(text: &str, pos: usize) -> usize {
    let mut at = line_start(text, pos);
    for want_blank in [true, false] {
        while at > 0 && blank_line(text, at) == want_blank {
            at = line_start(text, at - 1);
        }
    }
    at
}

/// `f`, `F`, `t`, `T`: where the `count`th `target` char is from `pos` on the same line, going
/// `forward` or back. With `till` it is the char before it (after it, going back). `repeat` is
/// for `;` and `,`, which skip a `till` match right beside the cursor instead of staying put.
pub fn find_in_line(
    text: &str,
    pos: usize,
    target: char,
    forward: bool,
    till: bool,
    count: usize,
    repeat: bool,
) -> Option<usize> {
    let skip_beside = till && repeat;
    let mut left = count;
    if forward {
        let beside = pos + text[pos..].chars().next().map_or(0, char::len_utf8);
        let line = &text[pos..line_end(text, pos)];
        for (i, c) in line.char_indices().skip(1) {
            let at = pos + i;
            if c != target || (skip_beside && at == beside) {
                continue;
            }
            left -= 1;
            if left == 0 {
                let before = text[..at].chars().next_back().map_or(0, char::len_utf8);
                return Some(if till { at - before } else { at });
            }
        }
    } else {
        let start = line_start(text, pos);
        for (i, c) in text[start..pos].char_indices().rev() {
            let at = start + i;
            if c != target || (skip_beside && at + c.len_utf8() == pos) {
                continue;
            }
            left -= 1;
            if left == 0 {
                return Some(if till { at + c.len_utf8() } else { at });
            }
        }
    }
    None
}

/// A space or tab. Text objects treat these as the only blanks; a newline always ends the object.
fn blank(c: char) -> bool {
    c == ' ' || c == '\t'
}

/// The range a text object covers around `pos`, and whether it is whole lines. `obj` is the
/// key after `i` or `a`: `w`, `p`, a quote (`"` `'` `` ` ``), or a bracket (either side, or `b`
/// `B`). `None` when there is no such object at `pos`.
pub fn text_object(
    text: &str,
    pos: usize,
    obj: char,
    around: bool,
) -> Option<(Range<usize>, bool)> {
    match obj {
        'w' => word_object(text, pos, around).map(|r| (r, false)),
        'p' => paragraph_object(text, pos, around).map(|r| (r, true)),
        '"' | '\'' | '`' => quote_object(text, pos, obj, around).map(|r| (r, false)),
        '(' | ')' | 'b' => bracket_object(text, pos, '(', ')', around).map(|r| (r, false)),
        '[' | ']' => bracket_object(text, pos, '[', ']', around).map(|r| (r, false)),
        '{' | '}' | 'B' => bracket_object(text, pos, '{', '}', around).map(|r| (r, false)),
        '<' | '>' => bracket_object(text, pos, '<', '>', around).map(|r| (r, false)),
        _ => None,
    }
}

/// `iw`: the run of blanks or of same-class chars at `pos`. `aw`: a word plus the blanks after
/// it, or before it if none follow; blanks plus the word after them.
fn word_object(text: &str, pos: usize, around: bool) -> Option<Range<usize>> {
    let c = text[pos..].chars().next().filter(|&c| c != '\n')?;
    let kind = |c: char| if blank(c) { 0 } else { class(c) };
    let same = |run_class: u8, c: char| c != '\n' && kind(c) == run_class;
    let run_of = |pos: usize| {
        let run_class = kind(text[pos..].chars().next().unwrap());
        let mut start = pos;
        while let Some(c) = text[..start]
            .chars()
            .next_back()
            .filter(|&c| same(run_class, c))
        {
            start -= c.len_utf8();
        }
        let mut end = pos;
        while let Some(c) = text[end..].chars().next().filter(|&c| same(run_class, c)) {
            end += c.len_utf8();
        }
        start..end
    };
    let run = run_of(pos);
    if !around {
        return Some(run);
    }
    if blank(c) {
        return Some(match text[run.end..].chars().next() {
            Some(next) if next != '\n' => run.start..run_of(run.end).end,
            _ => run,
        });
    }
    let trailing = text[run.end..]
        .bytes()
        .take_while(|&b| b == b' ' || b == b'\t')
        .count();
    if trailing > 0 {
        return Some(run.start..run.end + trailing);
    }
    let leading = text[..run.start]
        .bytes()
        .rev()
        .take_while(|&b| b == b' ' || b == b'\t')
        .count();
    Some(run.start - leading..run.end)
}

/// `ip`: the lines around `pos` that are all blank or all not blank. `ap` adds the blank lines
/// after a paragraph (before it if none follow), or the paragraph after blank lines. A line of
/// only whitespace is blank. The range takes whole lines, newline included.
fn paragraph_object(text: &str, pos: usize, around: bool) -> Option<Range<usize>> {
    if text.is_empty() {
        return None;
    }
    let is_blank = |start: usize| text[start..line_end(text, start)].trim().is_empty();
    // Start of the line after the one at `start`, if there is one.
    let next_line = |start: usize| Some(line_end(text, start) + 1).filter(|&n| n < text.len());
    let here = line_start(text, pos.min(text.len() - 1));
    let kind = is_blank(here);
    let (mut first, mut last) = (here, here);
    while first > 0 && is_blank(line_start(text, first - 1)) == kind {
        first = line_start(text, first - 1);
    }
    while let Some(next) = next_line(last).filter(|&n| is_blank(n) == kind) {
        last = next;
    }
    if around {
        // The other kind of lines follows, if anything does.
        match next_line(last) {
            Some(mut next) => {
                last = next;
                while let Some(n) = next_line(next).filter(|&n| is_blank(n) == !kind) {
                    (next, last) = (n, n);
                }
            }
            None if !kind => {
                while first > 0 && is_blank(line_start(text, first - 1)) {
                    first = line_start(text, first - 1);
                }
            }
            None => {}
        }
    }
    Some(first..(line_end(text, last) + 1).min(text.len()))
}

/// `i"`: inside the quote pair on the line that holds `pos`, or the first pair after it. `a"`
/// adds the quotes and the blanks after them, or before them if none follow. Backslash-escaped
/// quotes don't count.
fn quote_object(text: &str, pos: usize, quote: char, around: bool) -> Option<Range<usize>> {
    let (start, end) = (line_start(text, pos), line_end(text, pos));
    let mut quotes = Vec::new();
    let mut escaped = false;
    for (i, c) in text[start..end].char_indices() {
        if c == quote && !escaped {
            quotes.push(start + i);
        }
        escaped = c == '\\' && !escaped;
    }
    let (open, close) = quotes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&[open, close]| (open, close))
        .find(|&(_, close)| pos <= close)?;
    if !around {
        return Some(open + quote.len_utf8()..close);
    }
    let after = close + quote.len_utf8();
    let trailing = text[after..end]
        .bytes()
        .take_while(|&b| b == b' ' || b == b'\t')
        .count();
    if trailing > 0 {
        return Some(open..after + trailing);
    }
    let leading = text[start..open]
        .bytes()
        .rev()
        .take_while(|&b| b == b' ' || b == b'\t')
        .count();
    Some(open - leading..after)
}

/// `i(`: inside the innermost bracket pair around `pos`, which may span lines. `a(` includes the
/// brackets. On a bracket, that bracket's pair.
fn bracket_object(
    text: &str,
    pos: usize,
    open: char,
    close: char,
    around: bool,
) -> Option<Range<usize>> {
    let start = if text[pos..].starts_with(open) {
        pos
    } else {
        let mut depth = 0;
        text[..pos].char_indices().rev().find_map(|(i, c)| {
            if c == close {
                depth += 1;
            } else if c == open {
                if depth == 0 {
                    return Some(i);
                }
                depth -= 1;
            }
            None
        })?
    };
    let inner = start + open.len_utf8();
    let mut depth = 0;
    let end = text[inner..].char_indices().find_map(|(i, c)| {
        if c == open {
            depth += 1;
        } else if c == close {
            if depth == 0 {
                return Some(inner + i);
            }
            depth -= 1;
        }
        None
    })?;
    Some(if around {
        start..end + close.len_utf8()
    } else {
        inner..end
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Applies `motion` `times` times from `from`, returning where each step lands.
    fn walk(text: &str, motion: fn(&str, usize) -> usize, from: usize, times: usize) -> Vec<usize> {
        let mut pos = from;
        (0..times)
            .map(|_| {
                pos = motion(text, pos);
                pos
            })
            .collect()
    }

    #[test]
    fn paragraphs_are_found_by_blank_lines() {
        let text = "one\ntwo\n\n\nthree\n  \nfour";
        // (from, `}`, `{`): a run of blank lines counts once, and `{` stops nearest the text.
        for (from, forward, back) in [
            (0, 8, 0),
            (5, 8, 0),
            (8, 16, 0),
            (10, 16, 9),
            (12, 16, 9),
            (17, 23, 9),
            (19, 23, 16),
        ] {
            assert_eq!(paragraph_forward(text, from), forward, "}} from {from}");
            assert_eq!(paragraph_back(text, from), back, "{{ from {from}");
        }
    }

    #[test]
    fn big_words_only_stop_at_blanks() {
        let text = "foo.bar baz-qux  end";
        assert_eq!(word_forward_by(text, 0, true), 8);
        assert_eq!(word_forward(text, 0), 3);
        assert_eq!(word_end_by(text, 0, true), 6);
        assert_eq!(word_back_by(text, 12, true), 8);
    }

    #[test]
    fn find_stops_on_the_line_and_till_stays_beside() {
        let text = "a,b,c,d\nx,y";
        assert_eq!(find_in_line(text, 0, ',', true, false, 1, false), Some(1));
        assert_eq!(find_in_line(text, 0, ',', true, false, 3, false), Some(5));
        assert_eq!(find_in_line(text, 0, ',', true, false, 4, false), None);
        assert_eq!(find_in_line(text, 0, ',', true, true, 2, false), Some(2));
        assert_eq!(find_in_line(text, 6, ',', false, false, 1, false), Some(5));
        assert_eq!(find_in_line(text, 6, ',', false, true, 1, false), Some(6));
        // `;` after `t,` from just before a comma jumps on to the next one.
        assert_eq!(find_in_line(text, 0, ',', true, true, 1, false), Some(0));
        assert_eq!(find_in_line(text, 0, ',', true, true, 1, true), Some(2));
        assert_eq!(find_in_line("é,é", 0, ',', true, false, 1, false), Some(2));
    }

    #[test]
    fn w_moves_by_word_class_and_stops_on_empty_lines() {
        // "foo", ".", "bar", "baz", then the empty line, then "end".
        let text = "foo.bar  baz\n\nend";
        assert_eq!(walk(text, word_forward, 0, 6), [3, 4, 9, 13, 14, 17]);
        // Multi-byte chars are single steps.
        assert_eq!(walk("é ü", word_forward, 0, 2), [3, 5]);
    }

    #[test]
    fn b_moves_back_to_word_starts_and_empty_lines() {
        let text = "foo.bar  baz\n\nend";
        assert_eq!(walk(text, word_back, 17, 6), [14, 13, 9, 4, 3, 0]);
        // From the middle of a word it goes to that word's start.
        assert_eq!(word_back("hello", 3), 0);
        assert_eq!(word_back("a\nb", 2), 0);
    }

    #[test]
    fn e_moves_to_word_ends_across_lines() {
        let text = "foo.bar  baz\n\nend";
        assert_eq!(walk(text, word_end, 0, 5), [2, 3, 6, 11, 16]);
        // With no next word it stays put.
        assert_eq!(word_end("ab  ", 0), 1);
        assert_eq!(word_end("ab  ", 1), 1);
        assert_eq!(word_end("ab", 1), 1);
        assert_eq!(word_end("", 0), 0);
    }

    #[test]
    fn word_run_end_stops_at_a_class_change() {
        assert_eq!(word_run_end("foo.bar", 1), 2);
        assert_eq!(word_run_end("foo.bar", 3), 3);
        assert_eq!(word_run_end("é é", 0), 0);
        assert_eq!(word_run_end("ab", 1), 1);
    }

    #[test]
    fn line_helpers() {
        let text = "a\n  b\n\nc\n";
        assert_eq!(first_non_blank(text, 3), 4);
        assert_eq!(first_non_blank("   ", 1), 3);
        assert_eq!(line_end(text, 2), 5);
        // Line 2 is empty. Past the end lands on the last real line, not after the final \n.
        assert_eq!(nth_line_start(text, 2), 6);
        assert_eq!(nth_line_start(text, 99), 7);
        assert_eq!(nth_line_start("a\nb", 99), 2);
    }

    /// The text a text object covers at the first occurrence of `at` in `text`, plus whether it
    /// is linewise.
    fn object<'a>(text: &'a str, at: &str, obj: char, around: bool) -> Option<(&'a str, bool)> {
        let pos = text.find(at).unwrap();
        text_object(text, pos, obj, around).map(|(r, lines)| (&text[r], lines))
    }

    #[test]
    fn word_objects() {
        let text = "foo bar.baz  qux";
        assert_eq!(object(text, "oo", 'w', false), Some(("foo", false)));
        assert_eq!(object(text, " bar", 'w', false), Some((" ", false)));
        assert_eq!(object(text, ".", 'w', false), Some((".", false)));
        assert_eq!(object(text, "  qux", 'w', false), Some(("  ", false)));
        assert_eq!(object(text, "foo", 'w', true), Some(("foo ", false)));
        assert_eq!(object(text, "bar", 'w', true), Some((" bar", false)));
        // No blanks after the last word, so the ones before it join.
        assert_eq!(object(text, "qux", 'w', true), Some(("  qux", false)));
        assert_eq!(object(text, " bar", 'w', true), Some((" bar", false)));
        assert_eq!(object(text, "  qux", 'w', true), Some(("  qux", false)));
        // A newline ends it, and there is nothing on an empty line.
        assert_eq!(object("a b\nc", "b", 'w', true), Some((" b", false)));
        assert_eq!(text_object("a\n\nb", 2, 'w', false), None);
        assert_eq!(text_object("é ü", 3, 'w', false), Some((3..5, false)));
    }

    #[test]
    fn paragraph_objects() {
        let text = "a\nb\n\n\nc\nd";
        assert_eq!(object(text, "a", 'p', false), Some(("a\nb\n", true)));
        assert_eq!(object(text, "b", 'p', true), Some(("a\nb\n\n\n", true)));
        // On blank lines: the run of them, or them plus the next paragraph.
        assert_eq!(text_object(text, 4, 'p', false), Some((4..6, true)));
        assert_eq!(text_object(text, 4, 'p', true), Some((4..text.len(), true)));
        // The last paragraph has none after it, so the blank lines before it join.
        assert_eq!(object(text, "d", 'p', false), Some(("c\nd", true)));
        assert_eq!(object(text, "d", 'p', true), Some(("\n\nc\nd", true)));
        // Whitespace-only lines are blank.
        assert_eq!(
            object("a\n  \nb\n", "a", 'p', true),
            Some(("a\n  \n", true))
        );
        assert_eq!(text_object("", 0, 'p', false), None);
    }

    #[test]
    fn quote_objects() {
        let text = r#"say "hi there" ok "again" \"x\" z"#;
        assert_eq!(object(text, "hi", '"', false), Some(("hi there", false)));
        assert_eq!(
            object(text, "hi", '"', true),
            Some((r#""hi there" "#, false))
        );
        // On a quote, in a pair; before any pair, the next one; between pairs, the next.
        assert_eq!(
            object(text, r#""hi"#, '"', false),
            Some(("hi there", false))
        );
        assert_eq!(object(text, "say", '"', false), Some(("hi there", false)));
        assert_eq!(object(text, "ok", '"', false), Some(("again", false)));
        // No closing quote at the end, so blanks before the quotes join.
        assert_eq!(object(r#"a "b""#, "b", '"', true), Some((r#" "b""#, false)));
        // Escaped quotes don't pair; after the last pair there is nothing.
        assert_eq!(object(text, r#"\"x"#, '"', false), None);
        assert_eq!(object("a 'b' c", "b", '\'', false), Some(("b", false)));
        assert_eq!(object("no quotes", "quotes", '"', false), None);
    }

    #[test]
    fn bracket_objects() {
        let text = "f(a, (b), c) g";
        assert_eq!(object(text, "a,", '(', false), Some(("a, (b), c", false)));
        assert_eq!(object(text, "a,", ')', true), Some(("(a, (b), c)", false)));
        assert_eq!(object(text, "b", 'b', false), Some(("b", false)));
        assert_eq!(object(text, ", c", 'b', false), Some(("a, (b), c", false)));
        // On either bracket, that bracket's pair.
        assert_eq!(object(text, "(b", '(', true), Some(("(b)", false)));
        assert_eq!(object(text, "), c", '(', true), Some(("(b)", false)));
        assert_eq!(object("x [a {b} c]", "b", '{', false), Some(("b", false)));
        assert_eq!(
            object("x [a {b} c]", "b", '[', false),
            Some(("a {b} c", false))
        );
        assert_eq!(object("x <a>", "a", '>', true), Some(("<a>", false)));
        // Across lines, and unbalanced or missing.
        assert_eq!(object("(a\nb)", "b", '(', false), Some(("a\nb", false)));
        assert_eq!(object("(a", "a", '(', false), None);
        assert_eq!(object("a) b", "b", '(', false), None);
        assert_eq!(object("abc", "b", '(', false), None);
        assert_eq!(object("abc", "b", 'x', false), None);
    }
}
