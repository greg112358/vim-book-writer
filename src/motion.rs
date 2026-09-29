//! Cursor motions as pure functions on text and a byte position.

/// 0 for whitespace, 1 for word chars, 2 for punctuation. `w`, `b`, and `e` stop where it changes.
fn class(c: char) -> u8 {
    if c.is_whitespace() {
        0
    } else if c.is_alphanumeric() || c == '_' {
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
    let mut chars = text[pos..].char_indices().peekable();
    let Some(&(_, first)) = chars.peek() else {
        return text.len();
    };
    let start_class = class(first);
    if start_class != 0 {
        while chars.next_if(|&(_, c)| class(c) == start_class).is_some() {}
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
    let mut i = pos;
    let word_class = loop {
        let Some(c) = text[..i].chars().next_back() else {
            return 0;
        };
        if !c.is_whitespace() {
            break class(c);
        }
        i -= c.len_utf8();
        if c == '\n' && (i == 0 || text.as_bytes()[i - 1] == b'\n') {
            return i;
        }
    };
    while let Some(c) = text[..i]
        .chars()
        .next_back()
        .filter(|&c| class(c) == word_class)
    {
        i -= c.len_utf8();
    }
    i
}

/// `e`: the last char of this word, or if already there, of the next one. Stays put if there
/// is no next word.
pub fn word_end(text: &str, pos: usize) -> usize {
    let mut chars = text[pos..]
        .char_indices()
        .skip(1)
        .skip_while(|&(_, c)| c.is_whitespace());
    let Some((offset, first)) = chars.next() else {
        return pos;
    };
    let word_class = class(first);
    let mut end = pos + offset;
    for (offset, c) in chars {
        if class(c) != word_class {
            break;
        }
        end = pos + offset;
    }
    end
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
}
