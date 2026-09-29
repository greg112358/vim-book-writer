use crate::style::Style;
use jotdown::{Container, Event, Parser};
use std::ops::Range;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpanKind {
    Heading(u16),
    Blockquote,
    ListItem,
    CodeBlock,
    ThematicBreak,
    Strong,
    Emphasis,
    Strikethrough,
    /// Inline code.
    Code,
    Link,
    /// `[text]{attributes}`
    Span,
}

/// A styled piece of the source. Spans nest, so ranges overlap.
#[derive(Debug, Clone, PartialEq)]
pub struct Span {
    pub kind: SpanKind,
    /// The whole span in the source, delimiters included.
    pub range: Range<usize>,
    /// The part between the delimiters. What `range` holds around it is markup.
    pub content: Range<usize>,
    /// From a `style` attribute; empty when the span has none.
    pub style: Style,
}

/// Parses djot into styled spans, in document order with parents before their children.
/// Djot writes strong as `*x*`, so `**x**` comes out as two nested `Strong` spans.
pub fn parse(src: &str) -> Vec<Span> {
    let mut spans: Vec<Span> = Vec::new();
    // Indices into `spans` of the containers still open.
    let mut open: Vec<usize> = Vec::new();
    // End of the last content seen. Block containers end here, since their own End events sit
    // after the trailing newline.
    let mut last_end = 0;
    for (event, range) in Parser::new(src).into_offset_iter() {
        match &event {
            Event::Blankline => continue,
            Event::Start(container, attributes) => {
                if let Some(kind) = span_kind(container) {
                    let mut content_start = range.end;
                    // Skip the space after `## `, `> ` and `- `.
                    if matches!(
                        kind,
                        SpanKind::Heading(_) | SpanKind::Blockquote | SpanKind::ListItem
                    ) {
                        content_start += src[content_start..]
                            .bytes()
                            .take_while(|b| matches!(b, b' ' | b'\t'))
                            .count();
                    }
                    open.push(spans.len());
                    spans.push(Span {
                        kind,
                        range: range.clone(),
                        content: content_start..content_start,
                        style: attributes
                            .get_value("style")
                            .map_or_else(Style::default, |css| Style::parse(&css.to_string())),
                    });
                }
            }
            Event::End(container) => {
                if let Some(kind) = span_kind(container) {
                    let span = &mut spans[open.pop().unwrap()];
                    if kind == SpanKind::CodeBlock {
                        // Ends with the closing fence, which is markup.
                        span.content.end = range.start;
                        span.range.end = range.end;
                        last_end = last_end.max(range.end);
                    } else if container.is_block() {
                        span.content.end = last_end.max(span.content.start);
                        span.range.end = span.content.end;
                    } else {
                        span.content.end = range.start;
                        span.range.end = range.end;
                    }
                }
            }
            Event::ThematicBreak(_) => spans.push(Span {
                kind: SpanKind::ThematicBreak,
                range: range.clone(),
                content: range.end..range.end,
                style: Style::default(),
            }),
            _ => {}
        }
        if !matches!(&event, Event::Start(c, _) | Event::End(c) if c.is_block()) {
            last_end = last_end.max(range.end);
        }
    }
    spans
}

fn span_kind(container: &Container) -> Option<SpanKind> {
    Some(match container {
        Container::Heading { level, .. } => SpanKind::Heading(*level),
        Container::Blockquote => SpanKind::Blockquote,
        Container::ListItem => SpanKind::ListItem,
        Container::CodeBlock { .. } => SpanKind::CodeBlock,
        Container::Strong => SpanKind::Strong,
        Container::Emphasis => SpanKind::Emphasis,
        Container::Delete => SpanKind::Strikethrough,
        Container::Verbatim => SpanKind::Code,
        Container::Link(..) => SpanKind::Link,
        Container::Span => SpanKind::Span,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use SpanKind::*;

    /// Each span as (kind, source text of its range, source text of its content).
    fn view(src: &str) -> Vec<(SpanKind, &str, &str)> {
        parse(src)
            .into_iter()
            .map(|s| (s.kind, &src[s.range], &src[s.content]))
            .collect()
    }

    #[test]
    fn heading_with_inline_strong() {
        assert_eq!(
            view("## Head *x*\n\ntext"),
            [
                (Heading(2), "## Head *x*", "Head *x*"),
                (Strong, "*x*", "x"),
            ]
        );
    }

    #[test]
    fn list_items_and_blockquote() {
        assert_eq!(
            view("- one\n- two\n\n1. three\n\n> quote _it_\n"),
            [
                (ListItem, "- one", "one"),
                (ListItem, "- two", "two"),
                (ListItem, "1. three", "three"),
                (Blockquote, "> quote _it_", "quote _it_"),
                (Emphasis, "_it_", "it"),
            ]
        );
    }

    #[test]
    fn code_link_strikethrough_and_break() {
        assert_eq!(
            view("`a` [t](http://x.y) {-d-}\n\n```rust\nlet x;\n```\n\n***\n"),
            [
                (Code, "`a`", "a"),
                (Link, "[t](http://x.y)", "t"),
                (Strikethrough, "{-d-}", "d"),
                (CodeBlock, "```rust\nlet x;\n```\n", "let x;\n"),
                (ThematicBreak, "***", ""),
            ]
        );
    }

    #[test]
    fn span_carries_its_parsed_style() {
        let src = r#"[text]{style="color:#c00;font-size:14pt;text-decoration:underline"} and [x]{style="font-size:;color:"}"#;
        let spans = parse(src);
        assert_eq!(spans.len(), 2);
        assert_eq!(
            spans[0].style,
            Style {
                color: Some([0xcc, 0, 0]),
                size_pt: Some(14.0),
                family: None,
                underline: true,
            }
        );
        assert_eq!(spans[1].style, Style::default());
        assert_eq!(&src[spans[1].content.clone()], "x");
    }

    #[test]
    fn nested_and_doubled_markers() {
        assert_eq!(
            view("*a _b_ c*"),
            [(Strong, "*a _b_ c*", "a _b_ c"), (Emphasis, "_b_", "b")]
        );
        // Djot has no `**`; it is strong inside strong.
        assert_eq!(
            view("**bold**"),
            [(Strong, "**bold**", "*bold*"), (Strong, "*bold*", "bold")]
        );
    }
}
