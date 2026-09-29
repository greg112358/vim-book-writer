use crate::buffer::Buffer;
use crate::djot::{Span, SpanKind, parse};
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

/// Cuts `range` into one piece per paragraph, since a span can't cross a blank line. Each piece
/// is trimmed of whitespace and of a leading block marker (`## `, `> `, `- `), which can't sit
/// inside a span either. Pieces left empty are dropped.
fn pieces(text: &str, spans: &[Span], range: Range<usize>) -> Vec<Range<usize>> {
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

/// Underlines the text in `range` with `[text]{style="text-decoration:underline"}`, or removes the
/// underline when all of it is already underlined. One undo step.
pub fn toggle_underline(buf: &mut Buffer, range: Range<usize>) {
    edit_style(buf, range, "text-decoration", "underline", true);
}

/// Sets the text color of `range`. One undo step.
pub fn set_color(buf: &mut Buffer, range: Range<usize>, [r, g, b]: [u8; 3]) {
    edit_style(
        buf,
        range,
        "color",
        &format!("#{r:02x}{g:02x}{b:02x}"),
        false,
    );
}

/// Sets the font size of `range` in points. Does nothing unless `pt` is positive. One undo step.
pub fn set_font_size(buf: &mut Buffer, range: Range<usize>, pt: f32) {
    if pt.is_finite() && pt > 0.0 {
        edit_style(buf, range, "font-size", &format!("{pt}pt"), false);
    }
}

/// Sets the font family of `range`. Characters that would break the attribute or the CSS are
/// dropped, and a name that is empty after that does nothing. One undo step.
pub fn set_font_family(buf: &mut Buffer, range: Range<usize>, name: &str) {
    let name: String = name
        .chars()
        .filter(|c| !c.is_control() && !"\"'\\;:,{}".contains(*c))
        .collect();
    let name = name.trim();
    if name.is_empty() {
        return;
    }
    let value = if name.chars().all(|c| c.is_alphanumeric() || c == '-') {
        name.to_string()
    } else {
        format!("'{name}'")
    };
    edit_style(buf, range, "font-family", &value, false);
}

/// Where a span's `{...}` attribute block sits and what its style attribute holds.
struct StyleTarget {
    span: Span,
    /// The whole `{...}` block.
    block: Range<usize>,
    /// The `style=...` token inside the block, if there is one.
    token: Option<Range<usize>>,
    css: String,
}

/// Sets CSS property `prop` to `value` on the text in `range`, or with `toggle`, removes it when
/// all of the text already has exactly that value. Text inside an existing `[..]{..}` span joins
/// that span's `style` attribute, keeping its other attributes and declarations, instead of
/// getting a nested span. A span left with nothing is unwrapped. One undo step.
fn edit_style(buf: &mut Buffer, range: Range<usize>, prop: &str, value: &str, toggle: bool) {
    let text = buf.text();
    let spans = parse(text);
    let plan: Vec<(Range<usize>, Option<StyleTarget>)> = pieces(text, &spans, range)
        .into_iter()
        .map(|piece| {
            let target = spans
                .iter()
                .rev()
                .find(|s| {
                    s.kind == SpanKind::Span
                        && s.range.start <= piece.start
                        && piece.end <= s.range.end
                })
                .and_then(|span| {
                    let open =
                        span.content.end + text[span.content.end..span.range.end].find('{')?;
                    let block = open..span.range.end;
                    let (token, css) = match find_style(&text[block.clone()]) {
                        Some((token, css)) => (Some(open + token.start..open + token.end), css),
                        None => (None, String::new()),
                    };
                    Some(StyleTarget {
                        span: span.clone(),
                        block,
                        token,
                        css,
                    })
                });
            (piece, target)
        })
        .collect();
    let remove = toggle
        && !plan.is_empty()
        && plan.iter().all(|(_, target)| {
            target.as_ref().is_some_and(|t| {
                t.css.split(';').any(|declaration| {
                    declaration.split_once(':').is_some_and(|(k, v)| {
                        k.trim().eq_ignore_ascii_case(prop) && v.trim().eq_ignore_ascii_case(value)
                    })
                })
            })
        });

    buf.begin_group();
    // Last piece first, so the offsets of earlier pieces stay valid.
    for (piece, target) in plan.iter().rev() {
        match target {
            Some(t) => {
                let css = set_css(&t.css, prop, (!remove).then_some(value));
                let block = &buf.text()[t.block.clone()];
                let new_token = (!css.is_empty()).then(|| format!("style=\"{}\"", escape(&css)));
                let new_block = match (&t.token, new_token) {
                    (Some(token), Some(new)) => {
                        let (start, end) = (token.start - t.block.start, token.end - t.block.start);
                        format!("{}{new}{}", &block[..start], &block[end..])
                    }
                    (Some(token), None) => {
                        let (start, end) = (token.start - t.block.start, token.end - t.block.start);
                        format!("{}{}", block[..start].trim_end(), &block[end..])
                    }
                    (None, Some(new)) => {
                        let inner = block.strip_suffix('}').unwrap_or(block).trim_end();
                        let sep = if inner == "{" { "" } else { " " };
                        format!("{inner}{sep}{new}}}")
                    }
                    (None, None) => block.to_string(),
                };
                if new_block
                    .trim_matches(|c: char| c == '{' || c == '}' || c.is_whitespace())
                    .is_empty()
                {
                    buf.replace(t.span.content.end..t.span.range.end, "");
                    buf.replace(t.span.range.start..t.span.content.start, "");
                } else {
                    buf.replace(t.block.clone(), &new_block);
                }
            }
            None if !remove => {
                let css = set_css("", prop, Some(value));
                buf.replace(
                    piece.end..piece.end,
                    &format!("]{{style=\"{}\"}}", escape(&css)),
                );
                buf.replace(piece.start..piece.start, "[");
            }
            None => {}
        }
    }
    buf.end_group();
}

/// Finds the `style=...` attribute in a `{...}` block. Returns its range in the block and its
/// unescaped value.
fn find_style(block: &str) -> Option<(Range<usize>, String)> {
    let b = block.as_bytes();
    let mut i = 1;
    while i < b.len() {
        match b[i] {
            b'}' => break,
            c if c.is_ascii_whitespace() => i += 1,
            // `.class` and `#id`.
            b'.' | b'#' => {
                while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'}' {
                    i += 1;
                }
            }
            // `%comment%`.
            b'%' => {
                i += 1;
                while i < b.len() && b[i] != b'%' {
                    i += 1;
                }
                i += 1;
            }
            _ => {
                let start = i;
                while i < b.len() && !b[i].is_ascii_whitespace() && !matches!(b[i], b'=' | b'}') {
                    i += 1;
                }
                let key = &block[start..i];
                if i < b.len() && b[i] == b'=' {
                    i += 1;
                    let value_start = i;
                    if i < b.len() && b[i] == b'"' {
                        i += 1;
                        while i < b.len() && b[i] != b'"' {
                            i += if b[i] == b'\\' { 2 } else { 1 };
                        }
                        i += 1;
                    } else {
                        while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'}' {
                            i += 1;
                        }
                    }
                    let end = i.min(b.len());
                    if key == "style" {
                        let raw = block[value_start..end].trim_matches('"');
                        let mut css = String::with_capacity(raw.len());
                        let mut chars = raw.chars();
                        while let Some(c) = chars.next() {
                            match (c, chars.clone().next()) {
                                ('\\', Some(next)) if next.is_ascii_punctuation() => {
                                    css.push(next);
                                    chars.next();
                                }
                                _ => css.push(c),
                            }
                        }
                        return Some((start..end, css));
                    }
                }
            }
        }
    }
    None
}

/// Escapes a value for a double-quoted djot attribute.
fn escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Returns `css` with property `prop` set to `value`, or dropped when `value` is `None`. Other
/// declarations, known or not, stay in place.
fn set_css(css: &str, prop: &str, value: Option<&str>) -> String {
    let mut out = String::new();
    for declaration in css.split(';').map(str::trim).filter(|d| !d.is_empty()) {
        if declaration
            .split_once(':')
            .is_some_and(|(key, _)| key.trim().eq_ignore_ascii_case(prop))
        {
            continue;
        }
        if !out.is_empty() {
            out.push(';');
        }
        out.push_str(declaration);
    }
    if let Some(value) = value {
        if !out.is_empty() {
            out.push(';');
        }
        out.push_str(prop);
        out.push(':');
        out.push_str(value);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::Style;

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

    /// Underlines the first occurrence of `select` in `src`.
    fn underline(src: &str, select: &str) -> String {
        let start = src.find(select).unwrap();
        let mut buf = Buffer::new(src);
        toggle_underline(&mut buf, start..start + select.len());
        buf.text().to_string()
    }

    #[test]
    fn underline_wraps_in_a_style_span_and_toggles_off() {
        let on = r#"one [two]{style="text-decoration:underline"} three"#;
        assert_eq!(underline("one two three", "two"), on);
        assert_eq!(underline(on, "two"), "one two three");
    }

    #[test]
    fn underline_merges_into_an_existing_span() {
        // Keeps other declarations and toggles back to them.
        let both = r#"[t]{style="color:#c00;text-decoration:underline"}"#;
        assert_eq!(underline(r##"[t]{style="color:#c00"}"##, "t"), both);
        assert_eq!(underline(both, "t"), r##"[t]{style="color:#c00"}"##);
        // Unknown declarations survive.
        let bg = r#"[t]{style="background:red;text-decoration:underline"}"#;
        assert_eq!(underline(r#"[t]{style="background:red"}"#, "t"), bg);
        assert_eq!(underline(bg, "t"), r#"[t]{style="background:red"}"#);
        // Other attributes stay; a span with nothing left is unwrapped.
        let class = r#"[t]{.note style="text-decoration:underline"}"#;
        assert_eq!(underline("[t]{.note}", "t"), class);
        assert_eq!(underline(class, "t"), "[t]{.note}");
        assert_eq!(
            underline(r#"[t]{style="text-decoration:underline"}"#, "t"),
            "t"
        );
        // A different decoration is replaced, not treated as underlined.
        assert_eq!(
            underline(r#"[t]{style="text-decoration:line-through"}"#, "t"),
            r#"[t]{style="text-decoration:underline"}"#
        );
        // Quoted braces and spaces in another attribute don't confuse the search.
        let odd = r#"[t]{title="a } style=x" style="color:#000"}"#;
        assert_eq!(
            underline(odd, "t"),
            r##"[t]{title="a } style=x" style="color:#000;text-decoration:underline"}"##
        );
    }

    #[test]
    fn underline_handles_paragraphs_and_undo() {
        let src = "a\n\nb";
        let mut buf = Buffer::new(src);
        toggle_underline(&mut buf, 0..src.len());
        let on = r#"[a]{style="text-decoration:underline"}

[b]{style="text-decoration:underline"}"#;
        assert_eq!(buf.text(), on);
        assert!(buf.undo());
        assert_eq!(buf.text(), src);
        toggle_underline(&mut buf, 0..src.len());
        let len = buf.text().len();
        toggle_underline(&mut buf, 0..len);
        assert_eq!(buf.text(), src);
    }

    #[test]
    fn color_size_and_font_share_one_span_and_replace_each_other() {
        let src = "one two three";
        let start = src.find("two").unwrap();
        let mut buf = Buffer::new(src);
        set_color(&mut buf, start..start + 3, [0xcc, 0, 0]);
        // The text moved; the selection is now the word inside the new span.
        let start = buf.text().find("two").unwrap();
        set_font_size(&mut buf, start..start + 3, 14.5);
        set_font_family(&mut buf, start..start + 3, "Times New Roman");
        assert_eq!(
            buf.text(),
            r##"one [two]{style="color:#cc0000;font-size:14.5pt;font-family:'Times New Roman'"} three"##
        );
        set_color(&mut buf, start..start + 3, [0, 0, 255]);
        assert_eq!(buf.text().matches('[').count(), 1);
        let spans = parse(buf.text());
        assert_eq!(spans.len(), 1);
        assert_eq!(
            spans[0].style,
            Style {
                color: Some([0, 0, 255]),
                size_pt: Some(14.5),
                family: Some("Times New Roman".into()),
                underline: false,
            }
        );
        // Each command is one undo step.
        assert!(buf.undo());
        assert_eq!(parse(buf.text())[0].style.color, Some([0xcc, 0, 0]));
    }

    #[test]
    fn font_family_is_sanitized_and_bad_size_is_ignored() {
        let mut buf = Buffer::new("a");
        set_font_family(&mut buf, 0..1, "\";}{ ");
        set_font_size(&mut buf, 0..1, 0.0);
        set_font_size(&mut buf, 0..1, f32::NAN);
        assert_eq!(buf.text(), "a");
        set_font_family(&mut buf, 0..1, "Fira\" Code;{x}");
        assert_eq!(buf.text(), r#"[a]{style="font-family:'Fira Codex'"}"#);
        assert_eq!(
            parse(buf.text())[0].style.family.as_deref(),
            Some("Fira Codex")
        );
        let mut buf = Buffer::new("a");
        set_font_family(&mut buf, 0..1, "Georgia");
        assert_eq!(buf.text(), r#"[a]{style="font-family:Georgia"}"#);
    }
}
