use crate::buffer::Buffer;
use crate::djot::{SpanKind, parse};
use std::ops::Range;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    Bold,
    Italic,
    Strikethrough,
}

impl Mark {
    fn kind(self) -> SpanKind {
        match self {
            Mark::Bold => SpanKind::Strong,
            Mark::Italic => SpanKind::Emphasis,
            Mark::Strikethrough => SpanKind::Strikethrough,
        }
    }

    fn delimiters(self) -> (&'static str, &'static str) {
        match self {
            Mark::Bold => ("*", "*"),
            Mark::Italic => ("_", "_"),
            Mark::Strikethrough => ("{-", "-}"),
        }
    }
}

/// Wraps the text in `range` with the mark's delimiters, or takes the delimiters off when all of
/// it is already marked. Taking off removes the whole enclosing span, not just the selected part.
///
/// The range is cut into one piece per paragraph, since a span can't cross a blank line. Each
/// piece is trimmed of whitespace and of a leading block marker (`## `, `> `, `- `), which
/// can't sit inside a span either. One undo step.
pub fn toggle(buf: &mut Buffer, range: Range<usize>, mark: Mark) {
    let kind = mark.kind();
    let text = buf.text();
    let spans = parse(text);

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
        for span in &spans {
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
}
