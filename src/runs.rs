use crate::djot::{Span, SpanKind};
use std::ops::Range;

/// How a run of source text is drawn.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Format {
    /// Heading level, 0 outside a heading.
    pub heading: u16,
    pub quote: bool,
    pub code_block: bool,
    /// Thematic break.
    pub rule: bool,
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    pub superscript: bool,
    pub subscript: bool,
    /// Inline code.
    pub code: bool,
    pub link: bool,
    /// The `- ` or `1. ` that starts a list item.
    pub marker: bool,
}

/// A stretch of source text drawn one way.
#[derive(Debug, Clone, PartialEq)]
pub struct Run {
    pub range: Range<usize>,
    /// Syntax the writer typed rather than text a reader sees: `**`, `## `, `> `, code fences,
    /// and the `](url)` or `]{.class}` after a link or span. List markers count as text.
    /// It is drawn faint.
    pub markup: bool,
    /// From every span whose range holds the run, delimiters included, so `**` is drawn bold.
    pub format: Format,
}

/// Splits the whole source into runs that alternate between markup and content as the
/// spans open and close. Runs are in order, gap-free, and never empty; neighbours that look
/// the same are merged.
pub fn runs(src: &str, spans: &[Span]) -> Vec<Run> {
    // What happens at a position: 0 span opens, 1 content starts, 2 content ends, 3 span closes.
    let mut events: Vec<(usize, usize, u8)> = Vec::with_capacity(spans.len() * 4);
    for (i, span) in spans.iter().enumerate() {
        events.push((span.range.start, i, 0));
        events.push((span.content.start, i, 1));
        events.push((span.content.end, i, 2));
        events.push((span.range.end, i, 3));
    }
    events.sort_unstable();
    // The sentinel closes the last run.
    events.push((src.len(), usize::MAX, 0));

    let mut out: Vec<Run> = Vec::new();
    // Indices of the spans holding the current position, outermost first.
    let mut active: Vec<usize> = Vec::new();
    // Spans whose delimiters hold the current position, and list items whose marker does.
    let mut in_markup = 0i32;
    let mut in_marker = 0i32;
    let mut at = 0;
    for (pos, i, step) in events {
        if pos > at {
            let mut format = Format::default();
            for span in active.iter().map(|&j| &spans[j]) {
                match span.kind {
                    SpanKind::Heading(level) => format.heading = level,
                    SpanKind::Blockquote => format.quote = true,
                    SpanKind::CodeBlock => format.code_block = true,
                    SpanKind::ThematicBreak => format.rule = true,
                    SpanKind::Strong => format.bold = true,
                    SpanKind::Emphasis => format.italic = true,
                    SpanKind::Strikethrough => format.strike = true,
                    SpanKind::Superscript => format.superscript = true,
                    SpanKind::Subscript => format.subscript = true,
                    SpanKind::Code => format.code = true,
                    SpanKind::Link => format.link = true,
                    SpanKind::ListItem | SpanKind::BlockquoteMarker | SpanKind::Span => {}
                }
            }
            format.marker = in_marker > 0;
            let markup = in_markup > 0;
            match out.last_mut() {
                Some(last) if last.markup == markup && last.format == format => {
                    last.range.end = pos;
                }
                _ => out.push(Run {
                    range: at..pos,
                    markup,
                    format,
                }),
            }
            at = pos;
        }
        let Some(span) = spans.get(i) else { continue };
        // A list marker is text the reader sees, so it is marked rather than counted as markup.
        let list = span.kind == SpanKind::ListItem;
        let (d, m) = (i32::from(!list), i32::from(list));
        match step {
            0 => {
                active.insert(active.partition_point(|&j| j < i), i);
                in_markup += d;
                in_marker += m;
            }
            1 => {
                in_markup -= d;
                in_marker -= m;
            }
            2 => in_markup += d,
            _ => {
                active.retain(|&j| j != i);
                in_markup -= d;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::djot::parse;

    /// Each run as (is markup, source text).
    fn view(src: &str) -> Vec<(bool, &str)> {
        runs(src, &parse(src))
            .into_iter()
            .map(|r| (r.markup, &src[r.range]))
            .collect()
    }

    #[test]
    fn delimiters_are_markup_and_words_are_content() {
        assert_eq!(
            view("**Bold Text** and _italic_"),
            [
                (true, "**"),
                (false, "Bold Text"),
                (true, "**"),
                (false, " and "),
                (true, "_"),
                (false, "italic"),
                (true, "_"),
            ]
        );
        let src = "a *b _c_ d*";
        let runs = runs(src, &parse(src));
        let format = |text: &str| {
            let run = runs.iter().find(|r| &src[r.range.clone()] == text).unwrap();
            (run.format.bold, run.format.italic)
        };
        assert_eq!(format("a "), (false, false));
        assert_eq!(format("b "), (true, false));
        assert_eq!(format("c"), (true, true));
    }

    #[test]
    fn an_attribute_block_is_markup_and_the_text_is_not() {
        let src = r#"x [text]{.note} y"#;
        assert_eq!(
            view(src),
            [
                (false, "x "),
                (true, "["),
                (false, "text"),
                (true, "]{.note}"),
                (false, " y"),
            ]
        );
    }

    #[test]
    fn block_markers_and_list_markers() {
        let src = "## Title\n\n> quote\n\n- item\n\n***\n";
        assert_eq!(
            view(src),
            [
                (true, "## "),
                (false, "Title"),
                (false, "\n\n"),
                (true, "> "),
                (false, "quote"),
                (false, "\n\n"),
                (false, "- "),
                (false, "item\n\n"),
                (true, "***"),
                (false, "\n"),
            ]
        );
        let runs = runs(src, &parse(src));
        assert_eq!(runs[0].format.heading, 2);
        assert_eq!(runs[1].format.heading, 2);
        assert_eq!(runs[2].format.heading, 0);
        assert!(runs[3].format.quote && runs[4].format.quote);
        assert!(!runs[5].format.quote);
        // The list marker is the reader's text, in a look of its own.
        assert!(runs[6].format.marker && !runs[6].markup && !runs[7].format.marker);
    }

    #[test]
    fn covers_the_whole_source_without_gaps_or_empties() {
        let src = "# H\n\n```\ncode\n```\n\n*a*{style=\"color:#000\"} é [l](u)\n";
        let all = runs(src, &parse(src));
        assert_eq!(all.first().unwrap().range.start, 0);
        assert_eq!(all.last().unwrap().range.end, src.len());
        for pair in all.windows(2) {
            assert_eq!(pair[0].range.end, pair[1].range.start);
        }
        assert!(all.iter().all(|r| !r.range.is_empty()));
        assert!(runs("", &[]).is_empty());
    }
}
