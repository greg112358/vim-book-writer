use crate::buffer::Buffer;
use crate::djot::{Span, SpanKind, parse};
use crate::runs::runs;
use std::ops::Range;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    Bold,
    Italic,
    Strikethrough,
    Superscript,
    Subscript,
}

impl Mark {
    fn kind(self) -> SpanKind {
        match self {
            Mark::Bold => SpanKind::Strong,
            Mark::Italic => SpanKind::Emphasis,
            Mark::Strikethrough => SpanKind::Strikethrough,
            Mark::Superscript => SpanKind::Superscript,
            Mark::Subscript => SpanKind::Subscript,
        }
    }

    fn delimiters(self) -> (&'static str, &'static str) {
        match self {
            Mark::Bold => ("*", "*"),
            Mark::Italic => ("_", "_"),
            Mark::Strikethrough => ("{-", "-}"),
            Mark::Superscript => ("{^", "^}"),
            Mark::Subscript => ("{~", "~}"),
        }
    }
}

/// The word at `pos`: the one holding the char there, else the one ending just before `pos`, so
/// a caret at the end of a word still finds it. Apostrophes join a word but not at its ends.
/// Empty if there is no word.
fn word_at(text: &str, pos: usize) -> Range<usize> {
    let is_word = |c: char| c.is_alphanumeric() || c == '\'' || c == '\u{2019}';
    let anchor = match text[pos..].chars().next() {
        Some(c) if is_word(c) => pos,
        _ => match text[..pos].chars().next_back() {
            Some(c) if is_word(c) => pos - c.len_utf8(),
            _ => return pos..pos,
        },
    };
    let start = text[..anchor].trim_end_matches(is_word).len();
    let end = anchor
        + text[anchor..]
            .find(|c| !is_word(c))
            .unwrap_or(text.len() - anchor);
    let quote = ['\'', '\u{2019}'];
    let word = text[start..end].trim_start_matches(quote);
    let start = end - word.len();
    let word = word.trim_end_matches(quote);
    start..start + word.len()
}

/// Cuts `range` into one piece per paragraph, since a span can't cross a blank line. Each piece
/// is trimmed of whitespace and of a leading block marker (`## `, `> `, `- `), which can't sit
/// inside a span either. Pieces left empty are dropped. An empty `range` is a caret, and stands
/// for the word at it.
fn pieces(text: &str, spans: &[Span], range: Range<usize>) -> Vec<Range<usize>> {
    let range = if range.is_empty() {
        word_at(text, range.start)
    } else {
        range
    };
    let mut pieces: Vec<Range<usize>> = Vec::new();
    let mut piece_start = range.start;
    let mut pos = range.start;
    for line in text[range.clone()].split_inclusive('\n') {
        if line.trim().is_empty() {
            pieces.push(piece_start..pos);
            piece_start = pos + line.len();
        }
        pos += line.len();
    }
    pieces.push(piece_start..range.end);
    pieces.retain_mut(|piece| {
        let trimmed = &text[piece.clone()];
        let trimmed_start = piece.start + trimmed.len() - trimmed.trim_start().len();
        let trimmed_end = piece.end - (trimmed.len() - trimmed.trim_end().len());
        piece.start = trimmed_start;
        piece.end = trimmed_end;
        // Block spans are in document order, so nested markers are skipped in one pass.
        for span in spans {
            if matches!(
                span.kind,
                SpanKind::Heading(_)
                    | SpanKind::Blockquote
                    | SpanKind::BlockquoteMarker
                    | SpanKind::ListItem
            ) && span.range.start <= piece.start
                && piece.start < span.content.start
            {
                piece.start = span.content.start;
            }
        }
        piece.start < piece.end
    });
    pieces
}

/// Wraps the text in `range` with the mark's delimiters, or takes the delimiters off when all of
/// it is already marked. Taking off removes the whole enclosing span, not just the selected part.
///
/// Works per paragraph piece (see `pieces`). One undo step.
pub fn toggle(buf: &mut Buffer, range: Range<usize>, mark: Mark) {
    let kind = mark.kind();
    let text = buf.text();
    let spans = parse(text);

    let pieces = pieces(text, &spans, range);

    let marked = |piece: &Range<usize>| {
        spans
            .iter()
            .any(|s| s.kind == kind && s.range.start <= piece.start && piece.end <= s.range.end)
    };
    let all_marked = !pieces.is_empty() && pieces.iter().all(marked);

    buf.begin_group();
    // Last piece first, so the offsets of earlier pieces stay valid.
    for piece in pieces.iter().rev() {
        if all_marked {
            let (mut start, mut end) = (piece.start, piece.end);
            // Loops because `**x**` is strong inside strong.
            while let Some(span) = parse(buf.text())
                .into_iter()
                .rev()
                .find(|s| s.kind == kind && s.range.start <= start && end <= s.range.end)
            {
                // Closing delimiter first, so the opening one's offsets stay valid.
                buf.replace(span.content.end..span.range.end, "");
                buf.replace(span.range.start..span.content.start, "");
                let opening = span.content.start - span.range.start;
                start = start.clamp(span.content.start, span.content.end) - opening;
                end = end.clamp(span.content.start, span.content.end) - opening;
            }
        } else if !marked(piece) {
            let (open, close) = mark.delimiters();
            buf.replace(piece.end..piece.end, close);
            buf.replace(piece.start..piece.start, open);
        }
    }
    buf.end_group();
}

/// Uppercases the characters in `range`. It edits the text rather than styling it, so it survives
/// export. Markup inside the range (URLs, attribute blocks, fences) is left alone, since
/// uppercasing it would break links and styles. An empty `range` is a caret, and stands for the
/// word at it. One undo step, or none if nothing changes.
pub fn uppercase(buf: &mut Buffer, range: Range<usize>) {
    let text = buf.text();
    let range = if range.is_empty() {
        word_at(text, range.start)
    } else {
        range
    };
    let edits: Vec<(Range<usize>, String)> = runs(text, &parse(text))
        .into_iter()
        .filter(|run| !run.markup && run.range.start < range.end && range.start < run.range.end)
        .filter_map(|run| {
            let part = run.range.start.max(range.start)..run.range.end.min(range.end);
            let upper = text[part.clone()].to_uppercase();
            (upper != text[part.clone()]).then_some((part, upper))
        })
        .collect();
    buf.begin_group();
    // Last first, so the offsets of earlier edits stay valid.
    for (part, upper) in edits.iter().rev() {
        buf.replace(part.clone(), upper);
    }
    buf.end_group();
}

/// Makes the text in `range` a link with no target yet, `[text]()`, and returns where the cursor
/// goes to type it: inside the parens. If the text is already inside a link, the link comes off
/// and the result is `None`. Without any text it is `[]()` with the cursor in the brackets. One
/// undo step.
pub fn toggle_link(buf: &mut Buffer, range: Range<usize>) -> Option<usize> {
    let text = buf.text();
    let spans = parse(text);
    let Some(piece) = pieces(text, &spans, range.clone()).into_iter().next() else {
        buf.replace(range.start..range.start, "[]()");
        return Some(range.start + 1);
    };
    let link = spans.iter().rev().find(|s| {
        s.kind == SpanKind::Link && s.range.start <= piece.start && piece.end <= s.range.end
    });
    let unwrapped = link.map(|span| (span.range.clone(), text[span.content.clone()].to_string()));
    buf.begin_group();
    let cursor = match unwrapped {
        Some((whole, inner)) => {
            buf.replace(whole, &inner);
            None
        }
        None => {
            buf.replace(piece.end..piece.end, "]()");
            buf.replace(piece.start..piece.start, "[");
            Some(piece.end + 3)
        }
    };
    buf.end_group();
    cursor
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Applies `mark` to the first occurrence of `select` in `src`.
    fn apply(src: &str, select: &str, mark: Mark) -> String {
        let start = src.find(select).unwrap();
        let mut buf = Buffer::new(src);
        toggle(&mut buf, start..start + select.len(), mark);
        buf.text().to_string()
    }

    #[test]
    fn wraps_with_each_marks_delimiters() {
        assert_eq!(apply("one two", "two", Mark::Bold), "one *two*");
        assert_eq!(apply("one two", "two", Mark::Italic), "one _two_");
        assert_eq!(apply("one two", "two", Mark::Strikethrough), "one {-two-}");
        assert_eq!(apply("x 2", "2", Mark::Superscript), "x {^2^}");
        assert_eq!(apply("H 2", "2", Mark::Subscript), "H {~2~}");
        assert_eq!(apply("x {^2^}", "2", Mark::Superscript), "x 2");
    }

    #[test]
    fn removes_delimiters_when_already_applied() {
        assert_eq!(apply("one *two* three", "two", Mark::Bold), "one two three");
        // Selecting the delimiters too.
        assert_eq!(apply("one _two_ x", "_two_", Mark::Italic), "one two x");
        // Part of a longer span: the whole span goes.
        assert_eq!(apply("*one two three*", "two", Mark::Bold), "one two three");
        // Djot has no `**`, so this is strong inside strong and both layers come off.
        assert_eq!(apply("**bold**", "bold", Mark::Bold), "bold");
        assert_eq!(apply("{-gone-}", "gone", Mark::Strikethrough), "gone");
        // A different mark doesn't count as applied.
        assert_eq!(apply("*two*", "two", Mark::Italic), "*_two_*");
    }

    #[test]
    fn keeps_whitespace_and_block_markers_outside_the_delimiters() {
        assert_eq!(
            apply("one  two  three", " two ", Mark::Bold),
            "one  *two*  three"
        );
        assert_eq!(
            apply("## Title\n", "## Title\n", Mark::Bold),
            "## *Title*\n"
        );
        assert_eq!(apply("> - item", "> - item", Mark::Italic), "> - _item_");
        assert_eq!(apply("a  b", "  ", Mark::Bold), "a  b");
    }

    #[test]
    fn each_paragraph_is_wrapped_on_its_own() {
        assert_eq!(
            apply("a\nb\n\n\nc d\n", "a\nb\n\n\nc d", Mark::Bold),
            "*a\nb*\n\n\n*c d*\n"
        );
        // Mixed: only the unmarked paragraph changes.
        assert_eq!(apply("*a*\n\nb", "*a*\n\nb", Mark::Bold), "*a*\n\n*b*");
        // All marked: every paragraph is cleared.
        assert_eq!(apply("*a*\n\n*b*", "*a*\n\n*b*", Mark::Bold), "a\n\nb");
    }

    #[test]
    fn is_a_single_undo_step() {
        let mut buf = Buffer::new("a\n\nb");
        toggle(&mut buf, 0..4, Mark::Bold);
        assert_eq!(buf.text(), "*a*\n\n*b*");
        assert!(buf.undo());
        assert_eq!(buf.text(), "a\n\nb");
        assert!(!buf.undo());
    }

    #[test]
    fn uppercase_changes_text_but_not_markup() {
        let src = r#"héllo *world* ß [link](http://x.y) and [c]{.note}"#;
        let mut buf = Buffer::new(src);
        uppercase(&mut buf, 0..src.len());
        assert_eq!(
            buf.text(),
            r#"HÉLLO *WORLD* SS [LINK](http://x.y) AND [C]{.note}"#
        );
        assert!(buf.undo());
        assert_eq!(buf.text(), src);
        assert!(!buf.undo());
        // A partial selection stops where it stops, even inside a span.
        let mut buf = Buffer::new("hello *world*");
        uppercase(&mut buf, 2..9);
        assert_eq!(buf.text(), "heLLO *WOrld*");
        // Nothing to change means no undo step.
        let mut buf = Buffer::new("ABC 123");
        uppercase(&mut buf, 0..7);
        assert!(!buf.undo());
    }

    #[test]
    fn empty_range_formats_the_word_at_the_caret() {
        let at = |src: &str, pos: usize, f: fn(&mut Buffer, Range<usize>)| {
            let mut buf = Buffer::new(src);
            f(&mut buf, pos..pos);
            buf.text().to_string()
        };
        let bold = |buf: &mut Buffer, r| toggle(buf, r, Mark::Bold);
        // Inside a word, at its end, and at its start.
        assert_eq!(at("one two three", 5, bold), "one *two* three");
        assert_eq!(at("one two three", 7, bold), "one *two* three");
        assert_eq!(at("one two three", 4, bold), "one *two* three");
        // Apostrophes join a word, except at its ends.
        assert_eq!(at("don't stop", 2, bold), "*don't* stop");
        assert_eq!(at("'hi' there", 2, bold), "'*hi*' there");
        // No word, or a caret beside a word inside markup, still resolves.
        assert_eq!(at("a  b", 2, bold), "a  b");
        assert_eq!(at("", 0, bold), "");
        assert_eq!(at("one *two* three", 6, bold), "one two three");
        // The other commands use the same rule.
        assert_eq!(at("hello world", 8, uppercase), "hello WORLD");
    }

    #[test]
    fn every_command_is_one_undo_step_however_many_edits() {
        let src = "**a** b\n\nc [d]{.note}";
        let commands: [fn(&mut Buffer, Range<usize>); 6] = [
            |b, r| toggle(b, r, Mark::Bold),
            |b, r| toggle(b, r, Mark::Italic),
            |b, r| toggle(b, r, Mark::Strikethrough),
            |b, r| toggle(b, r, Mark::Superscript),
            |b, r| toggle(b, r, Mark::Subscript),
            uppercase,
        ];
        for command in commands {
            let mut buf = Buffer::new(src);
            command(&mut buf, 0..src.len());
            assert_ne!(buf.text(), src);
            assert!(buf.undo());
            assert_eq!(buf.text(), src);
            assert!(!buf.undo());
        }
    }

    #[test]
    fn link_wraps_the_word_and_puts_the_cursor_in_the_parens() {
        let mut buf = Buffer::new("see word here");
        assert_eq!(toggle_link(&mut buf, 6..6), Some(11));
        assert_eq!(buf.text(), "see [word]() here");
        assert_eq!(&buf.text()[..11], "see [word](");
        assert!(buf.undo());
        assert_eq!(buf.text(), "see word here");
        assert!(!buf.undo());
        // A selection, and only its first paragraph.
        let mut buf = Buffer::new("a b\n\nc");
        assert_eq!(toggle_link(&mut buf, 0..6), Some(6));
        assert_eq!(buf.text(), "[a b]()\n\nc");
        // Inside a link, it comes off and leaves the text.
        let mut buf = Buffer::new("x [text](http://a.b) y");
        assert_eq!(toggle_link(&mut buf, 4..8), None);
        assert_eq!(buf.text(), "x text y");
        // With no word, an empty link with the cursor in the brackets.
        let mut buf = Buffer::new("a  b");
        assert_eq!(toggle_link(&mut buf, 2..2), Some(3));
        assert_eq!(buf.text(), "a []() b");
    }
}
