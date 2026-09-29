//! Commands that work on whole lines: headings, quotes, and lists.

use crate::buffer::Buffer;
use std::ops::Range;

/// The lines `range` touches, each as its range without the newline. An empty range is the
/// caret's line. A range that ends right after a newline doesn't touch the line after it.
fn lines(text: &str, range: Range<usize>) -> Vec<Range<usize>> {
    let start = text[..range.start].rfind('\n').map_or(0, |i| i + 1);
    let last = if range.end > range.start && text[..range.end].ends_with('\n') {
        range.end - 1
    } else {
        range.end
    };
    let end = text[last..].find('\n').map_or(text.len(), |i| last + i);
    let mut lines = Vec::new();
    let mut at = start;
    while at <= end {
        let stop = text[at..end].find('\n').map_or(end, |i| at + i);
        lines.push(at..stop);
        at = stop + 1;
    }
    lines
}

fn is_blank(line: &str) -> bool {
    line.trim().is_empty()
}

/// The length of a heading marker (`## `) at the start of `line`, and its level.
fn heading(line: &str) -> Option<(usize, usize)> {
    let hashes = line.bytes().take_while(|&b| b == b'#').count();
    ((1..=6).contains(&hashes) && line[hashes..].starts_with(' ')).then_some((hashes + 1, hashes))
}

/// The length of a bullet (`- `) or number (`1. `) marker at the start of `line`, and whether it
/// is a number.
fn list_marker(line: &str) -> Option<(usize, bool)> {
    let digits = line.bytes().take_while(u8::is_ascii_digit).count();
    if digits > 0
        && matches!(line[digits..].chars().next(), Some('.' | ')'))
        && line[digits + 1..].starts_with(' ')
    {
        return Some((digits + 2, true));
    }
    (line.starts_with(['-', '*', '+']) && line[1..].starts_with(' ')).then_some((2, false))
}

/// Puts a blank line before and after the lines from `start` to `end` if they don't have one, so
/// a list or quote isn't read as part of the paragraph next to it. Edits from the bottom up.
fn set_apart(buf: &mut Buffer, start: usize, end: usize) {
    let text = buf.text();
    let after = end < text.len()
        && !is_blank(
            &text[end + 1
                ..text[end + 1..]
                    .find('\n')
                    .map_or(text.len(), |i| end + 1 + i)],
        );
    let before = start > 0
        && !is_blank(&text[text[..start - 1].rfind('\n').map_or(0, |i| i + 1)..start - 1]);
    if after {
        buf.replace(end + 1..end + 1, "\n");
    }
    if before {
        buf.replace(start..start, "\n");
    }
}

/// Makes the lines in `range` headings of `level` (1 to 6), or, if they all are already, plain
/// paragraphs. Blank lines are left alone. One undo step.
pub fn set_heading(buf: &mut Buffer, range: Range<usize>, level: usize) {
    let text = buf.text();
    let lines: Vec<Range<usize>> = lines(text, range)
        .into_iter()
        .filter(|l| !is_blank(&text[l.clone()]))
        .collect();
    let all_there = !lines.is_empty()
        && lines
            .iter()
            .all(|l| heading(&text[l.clone()]).is_some_and(|(_, at)| at == level));
    let marker = if all_there {
        String::new()
    } else {
        format!("{} ", "#".repeat(level))
    };
    buf.begin_group();
    for line in lines.iter().rev() {
        let old = heading(&buf.text()[line.clone()]).map_or(0, |(len, _)| len);
        buf.replace(line.start..line.start + old, &marker);
    }
    buf.end_group();
}

/// Quotes the lines in `range` with `> `, or, if they all are quoted already, takes one level of
/// quoting off. One undo step.
pub fn toggle_quote(buf: &mut Buffer, range: Range<usize>) {
    let text = buf.text();
    let lines = lines(text, range);
    let quoted = |line: &str| line.starts_with('>');
    let all_there = lines
        .iter()
        .filter(|l| !is_blank(&text[(*l).clone()]))
        .all(|l| quoted(&text[l.clone()]))
        && lines.iter().any(|l| !is_blank(&text[l.clone()]));
    let (first, last, len) = (lines[0].start, lines[lines.len() - 1].end, text.len());
    buf.begin_group();
    for line in lines.iter().rev() {
        let current = &buf.text()[line.clone()];
        if all_there {
            let len = if current.starts_with("> ") { 2 } else { 1 };
            buf.replace(line.start..line.start + len, "");
        } else if !quoted(current) {
            // A blank line inside a quote keeps the quote going with a bare `>`.
            buf.replace(
                line.start..line.start,
                if is_blank(current) { ">" } else { "> " },
            );
        }
    }
    if !all_there {
        // The edits were all inside the lines, so the last one ended as far along as they added.
        set_apart(buf, first, last + buf.text().len() - len);
    }
    buf.end_group();
}

/// Makes the lines in `range` items of a bullet list, or of a numbered one, or, if they all are
/// such items already, plain lines. Other markers are replaced. Blank lines are left alone. One
/// undo step.
pub fn toggle_list(buf: &mut Buffer, range: Range<usize>, numbered: bool) {
    let text = buf.text();
    let lines = lines(text, range);
    let items: Vec<Range<usize>> = lines
        .iter()
        .filter(|l| !is_blank(&text[(*l).clone()]))
        .cloned()
        .collect();
    let all_there = !items.is_empty()
        && items
            .iter()
            .all(|l| list_marker(&text[l.clone()]).is_some_and(|(_, n)| n == numbered));
    let (first, last, len) = (lines[0].start, lines[lines.len() - 1].end, text.len());
    buf.begin_group();
    for (i, line) in items.iter().enumerate().rev() {
        let old = list_marker(&buf.text()[line.clone()]).map_or(0, |(len, _)| len);
        let marker = match (all_there, numbered) {
            (true, _) => String::new(),
            (false, true) => format!("{}. ", i + 1),
            (false, false) => "- ".to_string(),
        };
        buf.replace(line.start..line.start + old, &marker);
    }
    if !all_there {
        set_apart(buf, first, last + buf.text().len() - len);
    }
    buf.end_group();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs `f` over the first occurrence of `select` in `src`, or the caret at its start if empty.
    fn run(src: &str, select: &str, f: impl FnOnce(&mut Buffer, Range<usize>)) -> String {
        let start = src.find(select).unwrap();
        let mut buf = Buffer::new(src);
        f(&mut buf, start..start + select.len());
        buf.text().to_string()
    }

    #[test]
    fn headings_are_set_replaced_and_toggled_off() {
        let heading = |src: &str, sel: &str, level| run(src, sel, |b, r| set_heading(b, r, level));
        assert_eq!(heading("Title\n", "Title", 1), "# Title\n");
        assert_eq!(heading("a\nb\n\nc", "a\nb", 2), "## a\n## b\n\nc");
        // The caret's line, another level replaces the marker, and the same level takes it off.
        assert_eq!(heading("one\ntwo\n", "two", 3), "one\n### two\n");
        assert_eq!(heading("# Title\n", "Title", 3), "### Title\n");
        assert_eq!(heading("## Title\n", "Title", 2), "Title\n");
        assert_eq!(heading("## a\nb", "a\nb", 2), "## a\n## b");
        // Hashes without a space aren't a marker, and blank lines stay blank.
        assert_eq!(heading("#tag\n", "tag", 1), "# #tag\n");
        assert_eq!(heading("a\n\nb", "a\n\nb", 1), "# a\n\n# b");
        assert_eq!(heading("", "", 1), "");
    }

    #[test]
    fn quotes_are_added_and_removed_a_level_at_a_time() {
        let quote = |src: &str, sel: &str| run(src, sel, toggle_quote);
        assert_eq!(quote("hello", "hello"), "> hello");
        assert_eq!(quote("> hello", "hello"), "hello");
        assert_eq!(quote(">hello", "hello"), "hello");
        assert_eq!(quote("> > deep", "deep"), "> deep");
        // Blank lines inside keep the quote going, and it is set apart from the text next to it.
        assert_eq!(quote("a\n\nb", "a\n\nb"), "> a\n>\n> b");
        assert_eq!(
            quote("intro\nquote me\noutro", "quote me"),
            "intro\n\n> quote me\n\noutro"
        );
        assert_eq!(quote("> a\nb", "a\nb"), "> a\n> b");
    }

    #[test]
    fn lists_are_bulleted_numbered_switched_and_removed() {
        let bullets = |src: &str, sel: &str| run(src, sel, |b, r| toggle_list(b, r, false));
        let numbers = |src: &str, sel: &str| run(src, sel, |b, r| toggle_list(b, r, true));
        assert_eq!(bullets("a\nb", "a\nb"), "- a\n- b");
        assert_eq!(bullets("- a\n- b", "a\n- b"), "a\nb");
        assert_eq!(numbers("a\nb\n\nc", "a\nb\n\nc"), "1. a\n2. b\n\n3. c");
        assert_eq!(numbers("1. a\n2. b", "a\n2. b"), "a\nb");
        // Switching kinds replaces the markers.
        assert_eq!(numbers("- a\n- b", "a\n- b"), "1. a\n2. b");
        assert_eq!(bullets("1. a\n2. b", "a\n2. b"), "- a\n- b");
        // Any bullet counts as one, so they come off; a mix of bullets and plain lines gets `-`.
        assert_eq!(bullets("* a\n+ b", "a\n+ b"), "a\nb");
        assert_eq!(bullets("* a\nb", "a\nb"), "- a\n- b");
        // Set apart from the paragraph around it, since a list can't interrupt one.
        assert_eq!(
            bullets("intro\nitem\noutro", "item"),
            "intro\n\n- item\n\noutro"
        );
        assert_eq!(
            bullets("intro\n\nitem\n\noutro", "item"),
            "intro\n\n- item\n\noutro"
        );
        assert_eq!(bullets("- a\n\ntext", "a"), "a\n\ntext");
        // Text that only looks like a marker isn't one.
        assert_eq!(bullets("-a", "-a"), "- -a");
        assert_eq!(bullets("2024.year", "2024"), "- 2024.year");
    }

    #[test]
    fn each_command_is_one_undo_step_and_the_djot_reads_back_right() {
        for f in [
            (|b: &mut Buffer, r| set_heading(b, r, 2)) as fn(&mut Buffer, Range<usize>),
            toggle_quote,
            |b, r| toggle_list(b, r, false),
            |b, r| toggle_list(b, r, true),
        ] {
            let src = "intro\nfirst\nsecond\noutro";
            let mut buf = Buffer::new(src);
            f(&mut buf, 6..17);
            assert_ne!(buf.text(), src);
            assert!(buf.undo());
            assert_eq!(buf.text(), src);
            assert!(!buf.undo());
        }
        // The result really is a list and a quote to a djot parser.
        let mut buf = Buffer::new("intro\nfirst\nsecond\noutro");
        toggle_list(&mut buf, 6..17, false);
        let kinds: Vec<_> = crate::djot::parse(buf.text())
            .iter()
            .map(|s| s.kind)
            .collect();
        assert_eq!(
            kinds
                .iter()
                .filter(|k| **k == crate::djot::SpanKind::ListItem)
                .count(),
            2
        );
        let mut buf = Buffer::new("intro\nquoted\noutro");
        toggle_quote(&mut buf, 6..12);
        let kinds: Vec<_> = crate::djot::parse(buf.text())
            .iter()
            .map(|s| s.kind)
            .collect();
        assert!(kinds.contains(&crate::djot::SpanKind::Blockquote));
    }
}
