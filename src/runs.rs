use crate::djot::{Span, SpanKind};
use crate::style::Style;
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
    /// Inline code.
    pub code: bool,
    pub link: bool,
    /// Merged from every enclosing span's style attribute; the innermost value wins.
    pub style: Style,
}

/// A stretch of source text drawn one way.
#[derive(Debug, Clone, PartialEq)]
pub struct Run {
    pub range: Range<usize>,
    /// Syntax the writer typed rather than text a reader sees: `**`, `## `, `> `, code fences,
    /// and the `](url)` or `]{style="..."}` after a link or span. List markers count as text.
    /// Insert mode draws markup faint; normal mode hides it.
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
    // Spans whose delimiters hold the current position.
    let mut in_markup = 0i32;
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
                    SpanKind::Code => format.code = true,
                    SpanKind::Link => format.link = true,
                    SpanKind::ListItem | SpanKind::BlockquoteMarker | SpanKind::Span => {}
                }
                format.style.color = span.style.color.or(format.style.color);
                format.style.size_pt = span.style.size_pt.or(format.style.size_pt);
                if span.style.family.is_some() {
                    format.style.family.clone_from(&span.style.family);
                }
                format.style.underline |= span.style.underline;
            }
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
        // A list marker is text, so list items never count as markup.
        let d = i32::from(span.kind != SpanKind::ListItem);
        match step {
            0 => {
                active.insert(active.partition_point(|&j| j < i), i);
                in_markup += d;
            }
            1 => in_markup -= d,
            2 => in_markup += d,
            _ => {
                active.remove(active.binary_search(&i).unwrap());
                in_markup -= d;
            }
        }
    }
    out
}

/// What normal mode shows: the runs that aren't markup, laid end to end.
#[derive(Debug)]
pub struct Visible<'a> {
    pub text: String,
    /// Each shown run as its range in `text` and the run it came from, which holds the
    /// source range and the format.
    pub runs: Vec<(Range<usize>, &'a Run)>,
}

/// Hides every markup run. `runs` must come from `runs()` on the same `src`.
pub fn visible<'a>(src: &str, runs: &'a [Run]) -> Visible<'a> {
    let mut text = String::new();
    let mut shown = Vec::new();
    for run in runs.iter().filter(|run| !run.markup) {
        let start = text.len();
        text.push_str(&src[run.range.clone()]);
        shown.push((start..text.len(), run));
    }
    Visible { text, runs: shown }
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
    fn style_attribute_block_is_markup_and_style_applies_to_content() {
        let src = r#"x [text]{style="color:#c00"} y"#;
        assert_eq!(
            view(src),
            [
                (false, "x "),
                (true, "["),
                (false, "text"),
                (true, r#"]{style="color:#c00"}"#),
                (false, " y"),
            ]
        );
        let runs = runs(src, &parse(src));
        assert_eq!(runs[0].format.style.color, None);
        assert_eq!(runs[2].format.style.color, Some([0xcc, 0, 0]));
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
                (false, "\n\n- item\n\n"),
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

    #[test]
    fn normal_mode_hides_delimiters_markers_and_attribute_blocks() {
        let check = |src: &str| {
            let runs = runs(src, &parse(src));
            visible(src, &runs).text
        };
        assert_eq!(check("**bold** and _it_"), "bold and it");
        assert_eq!(
            check(
                "## Head\n\n> a\n> b\n\n[t]{style=\"color:#c00\"} ok\n\n***\n\n```rust\nlet x;\n```\n\n- [l](http://x.y)\n"
            ),
            "Head\n\na\nb\n\nt ok\n\n\n\nlet x;\n\n- l\n"
        );
    }

    #[test]
    fn visible_runs_tile_the_text_and_keep_source_ranges() {
        let src = "é **b** ü";
        let runs = runs(src, &parse(src));
        let shown = visible(src, &runs);
        assert_eq!(shown.text, "é b ü");
        assert_eq!(shown.runs[1].0, 3..4);
        assert_eq!(shown.runs[1].1.range, 5..6);
        assert!(shown.runs[1].1.format.bold);
    }
}
