use crate::style::Style;
use jotdown::{Container, Event, ListKind, Parser};
use printpdf::{
    BuiltinFont, Color, Line, LinePoint, Mm, Op, PaintMode, PdfDocument, PdfFontHandle, PdfPage,
    PdfSaveOptions, Point, Pt, Rect, Rgb, TextItem,
};
use std::collections::HashMap;

const PAGE_WIDTH: f32 = 595.0;
const PAGE_HEIGHT: f32 = 842.0;
const MARGIN: f32 = 62.0;
const BODY_PT: f32 = 12.0;
const LIST_INDENT: f32 = 22.0;
const QUOTE_INDENT: f32 = 16.0;
const HEADING_PT: [f32; 7] = [BODY_PT, 24.0, 20.0, 16.0, 14.0, 12.0, 11.0];

/// A stretch of inline text with one look.
#[derive(Clone, Default)]
struct Piece {
    text: String,
    bold: bool,
    italic: bool,
    strike: bool,
    code: bool,
    link: bool,
    style: Style,
}

enum Kind {
    /// A paragraph, or a heading of the level given (0 for a paragraph).
    Text(u16),
    Code,
    Rule,
}

struct Block {
    kind: Kind,
    /// Left edge of the text, from the margin.
    indent: f32,
    quote_depth: usize,
    /// `•` or `1.` hung in front of the first line.
    marker: Option<String>,
    pieces: Vec<Piece>,
}

/// The look of a run on the page. Neighbouring words with the same one share a text operation.
#[derive(Clone, Copy, PartialEq)]
struct Look {
    font: BuiltinFont,
    size: f32,
    color: [u8; 3],
    underline: bool,
    strike: bool,
}

struct Word {
    text: String,
    look: Look,
    width: f32,
    space: bool,
}

/// Picks one of the 14 built-in PDF fonts: Times, Helvetica, or Courier, by what the family name
/// looks like, with bold and italic variants.
fn pick_font(family: Option<&str>, mono: bool, bold: bool, italic: bool) -> BuiltinFont {
    let name = family.unwrap_or("").to_lowercase();
    let has = |keys: &[&str]| keys.iter().any(|k| name.contains(k));
    let mono = mono || has(&["courier", "mono", "consolas", "menlo", "code"]);
    let sans = !mono
        && has(&[
            "sans",
            "arial",
            "helvetica",
            "verdana",
            "tahoma",
            "calibri",
            "segoe",
            "roboto",
            "inter",
            "lato",
        ]);
    use BuiltinFont::*;
    match (mono, sans, bold, italic) {
        (true, _, false, false) => Courier,
        (true, _, true, false) => CourierBold,
        (true, _, false, true) => CourierOblique,
        (true, _, true, true) => CourierBoldOblique,
        (_, true, false, false) => Helvetica,
        (_, true, true, false) => HelveticaBold,
        (_, true, false, true) => HelveticaOblique,
        (_, true, true, true) => HelveticaBoldOblique,
        (_, _, false, false) => TimesRoman,
        (_, _, true, false) => TimesBold,
        (_, _, false, true) => TimesItalic,
        (_, _, true, true) => TimesBoldItalic,
    }
}

/// Measures text in the built-in fonts, from the metrics printpdf ships with them.
#[derive(Default)]
struct Measure {
    fonts: HashMap<usize, printpdf::ParsedFont>,
}

impl Measure {
    fn width(&mut self, font: BuiltinFont, text: &str, size: f32) -> f32 {
        let parsed = self
            .fonts
            .entry(font.get_num())
            .or_insert_with(|| font.get_parsed_font().expect("built-in fonts always parse"));
        let unit = f32::from(parsed.units_per_em).max(1.0);
        // The subset fonts have no glyph for a space, so use the font's own space width.
        let space = match font {
            BuiltinFont::Courier
            | BuiltinFont::CourierBold
            | BuiltinFont::CourierOblique
            | BuiltinFont::CourierBoldOblique => 0.6,
            BuiltinFont::Helvetica
            | BuiltinFont::HelveticaBold
            | BuiltinFont::HelveticaOblique
            | BuiltinFont::HelveticaBoldOblique => 0.278,
            _ => 0.25,
        } * unit;
        let units: f32 = text
            .chars()
            .map(|c| {
                if c == ' ' || c == '\u{a0}' {
                    return space;
                }
                let glyph = parsed
                    .lookup_glyph_index(c as u32)
                    .or_else(|| parsed.lookup_glyph_index('?' as u32));
                f32::from(glyph.and_then(|g| parsed.get_glyph_width(g)).unwrap_or(500))
            })
            .sum();
        units * size / unit
    }
}

/// Walks the djot into blocks of styled text. Lists and quotes become indentation; anything outside
/// the v1 scope is written as plain paragraphs.
fn blocks(src: &str) -> Vec<Block> {
    let mut out: Vec<Block> = Vec::new();
    let mut current: Option<Block> = None;
    let mut quote_depth = 0;
    // One entry per open list: the number the next item gets, or `None` for bullets.
    let mut lists: Vec<Option<u64>> = Vec::new();
    let mut pending_marker: Option<String> = None;
    let (mut bold, mut italic, mut strike, mut code, mut link) = (0, 0, 0, 0, 0);
    let mut styles: Vec<Style> = Vec::new();

    for event in Parser::new(src) {
        let text: Option<String> = match event {
            Event::Start(container, attributes) => {
                match container {
                    Container::Paragraph | Container::Heading { .. } => {
                        let level = match container {
                            Container::Heading { level, .. } => level,
                            _ => 0,
                        };
                        current = Some(Block {
                            kind: Kind::Text(level),
                            indent: lists.len() as f32 * LIST_INDENT
                                + quote_depth as f32 * QUOTE_INDENT,
                            quote_depth,
                            marker: pending_marker.take(),
                            pieces: Vec::new(),
                        });
                    }
                    Container::CodeBlock { .. } => {
                        current = Some(Block {
                            kind: Kind::Code,
                            indent: lists.len() as f32 * LIST_INDENT
                                + quote_depth as f32 * QUOTE_INDENT,
                            quote_depth,
                            marker: pending_marker.take(),
                            pieces: Vec::new(),
                        });
                    }
                    Container::Blockquote => quote_depth += 1,
                    Container::List { kind, .. } => lists.push(match kind {
                        ListKind::Ordered { start, .. } => Some(start),
                        _ => None,
                    }),
                    Container::ListItem | Container::TaskListItem { .. } => {
                        pending_marker = Some(match lists.last_mut() {
                            Some(Some(n)) => {
                                *n += 1;
                                format!("{}.", *n - 1)
                            }
                            _ => "\u{2022}".to_string(),
                        });
                    }
                    Container::Strong => bold += 1,
                    Container::Emphasis => italic += 1,
                    Container::Delete => strike += 1,
                    Container::Verbatim => code += 1,
                    Container::Link(..) => link += 1,
                    Container::Span => styles.push(
                        attributes
                            .get_value("style")
                            .map_or_else(Style::default, |css| Style::parse(&css.to_string())),
                    ),
                    _ => {}
                }
                None
            }
            Event::End(container) => {
                match container {
                    Container::Paragraph
                    | Container::Heading { .. }
                    | Container::CodeBlock { .. } => {
                        out.extend(current.take().filter(|b| !b.pieces.is_empty()));
                    }
                    Container::Blockquote => quote_depth -= 1,
                    Container::List { .. } => {
                        lists.pop();
                    }
                    Container::Strong => bold -= 1,
                    Container::Emphasis => italic -= 1,
                    Container::Delete => strike -= 1,
                    Container::Verbatim => code -= 1,
                    Container::Link(..) => link -= 1,
                    Container::Span => {
                        styles.pop();
                    }
                    _ => {}
                }
                None
            }
            Event::Str(s) => Some(s.to_string()),
            Event::Softbreak => Some(" ".into()),
            Event::Hardbreak => Some("\n".into()),
            Event::NonBreakingSpace => Some("\u{a0}".into()),
            Event::Ellipsis => Some("…".into()),
            Event::EnDash => Some("–".into()),
            Event::EmDash => Some("—".into()),
            Event::LeftSingleQuote => Some("‘".into()),
            Event::RightSingleQuote => Some("’".into()),
            Event::LeftDoubleQuote => Some("“".into()),
            Event::RightDoubleQuote => Some("”".into()),
            Event::Symbol(name) => Some(format!(":{name}:")),
            Event::ThematicBreak(_) => {
                out.push(Block {
                    kind: Kind::Rule,
                    indent: lists.len() as f32 * LIST_INDENT + quote_depth as f32 * QUOTE_INDENT,
                    quote_depth,
                    marker: None,
                    pieces: Vec::new(),
                });
                None
            }
            _ => None,
        };
        if let (Some(text), Some(block)) = (text, current.as_mut()) {
            // Inner spans win over outer ones.
            let mut style = Style::default();
            for s in &styles {
                style.color = s.color.or(style.color);
                style.size_pt = s.size_pt.or(style.size_pt);
                if s.family.is_some() {
                    style.family.clone_from(&s.family);
                }
                style.underline |= s.underline;
            }
            block.pieces.push(Piece {
                text,
                bold: bold > 0,
                italic: italic > 0,
                strike: strike > 0,
                code: code > 0,
                link: link > 0,
                style,
            });
        }
    }
    out
}

fn look_of(piece: &Piece, heading: u16, kind: &Kind) -> Look {
    let base = HEADING_PT[heading.min(6) as usize];
    let mono = piece.code || matches!(kind, Kind::Code);
    let size = piece
        .style
        .size_pt
        .unwrap_or(if mono { base * 0.9 } else { base });
    Look {
        font: pick_font(
            piece.style.family.as_deref(),
            mono,
            piece.bold || heading > 0,
            piece.italic,
        ),
        size,
        color: piece
            .style
            .color
            .unwrap_or(if piece.link { [0, 0, 0xee] } else { [0, 0, 0] }),
        underline: piece.style.underline || piece.link,
        strike: piece.strike,
    }
}

fn rgb(color: [u8; 3]) -> Color {
    Color::Rgb(Rgb::new(
        f32::from(color[0]) / 255.0,
        f32::from(color[1]) / 255.0,
        f32::from(color[2]) / 255.0,
        None,
    ))
}

fn gray(level: f32) -> Color {
    Color::Rgb(Rgb::new(level, level, level, None))
}

fn point(x: f32, y: f32) -> Point {
    Point { x: Pt(x), y: Pt(y) }
}

fn stroke(from: (f32, f32), to: (f32, f32), color: Color, thickness: f32) -> [Op; 3] {
    [
        Op::SetOutlineColor { col: color },
        Op::SetOutlineThickness { pt: Pt(thickness) },
        Op::DrawLine {
            line: Line {
                points: vec![
                    LinePoint {
                        p: point(from.0, from.1),
                        bezier: false,
                    },
                    LinePoint {
                        p: point(to.0, to.1),
                        bezier: false,
                    },
                ],
                is_closed: false,
            },
        },
    ]
}

/// Fills pages from top to bottom, starting a new one when a line doesn't fit.
struct Pages {
    done: Vec<Vec<Op>>,
    ops: Vec<Op>,
    /// Top of the next line, measured up from the bottom of the page.
    y: f32,
}

impl Pages {
    fn room_for(&mut self, height: f32) {
        if self.y - height < MARGIN {
            self.done.push(std::mem::take(&mut self.ops));
            self.y = PAGE_HEIGHT - MARGIN;
        }
    }
}

/// Splits a word wider than `max` into pieces that fit.
fn split_wide(word: Word, max: f32, measure: &mut Measure) -> Vec<Word> {
    if word.width <= max {
        return vec![word];
    }
    let mut pieces: Vec<Word> = Vec::new();
    let mut text = String::new();
    for c in word.text.chars() {
        text.push(c);
        if measure.width(word.look.font, &text, word.look.size) > max && text.chars().count() > 1 {
            text.pop();
            let width = measure.width(word.look.font, &text, word.look.size);
            pieces.push(Word {
                text: std::mem::take(&mut text),
                look: word.look,
                width,
                space: false,
            });
            text.push(c);
        }
    }
    let width = measure.width(word.look.font, &text, word.look.size);
    pieces.push(Word {
        text,
        look: word.look,
        width,
        space: false,
    });
    pieces
}

/// Turns djot into a PDF that keeps its styling: headings, bold, italic, strikethrough, underline,
/// color, size, font family (as Times, Helvetica, or Courier), links, lists, quotes, and code. The
/// text is set in the 14 fonts every PDF reader has, so characters outside Windows-1252 come out as
/// `?`.
pub fn to_pdf(src: &str, title: &str) -> Vec<u8> {
    let mut measure = Measure::default();
    let mut pages = Pages {
        done: Vec::new(),
        ops: Vec::new(),
        y: PAGE_HEIGHT - MARGIN,
    };

    for block in blocks(src) {
        let left = MARGIN + block.indent;
        let avail = PAGE_WIDTH - MARGIN - left;
        let heading = match block.kind {
            Kind::Text(level) => level,
            _ => 0,
        };
        let at_top = pages.y >= PAGE_HEIGHT - MARGIN - 0.5;
        if !at_top {
            pages.y -= if heading > 0 {
                HEADING_PT[heading.min(6) as usize] * 0.7
            } else {
                2.0
            };
        }

        // Lines of words, each with the height it needs.
        let mut lines: Vec<Vec<Word>> = Vec::new();
        match block.kind {
            Kind::Rule => {
                pages.room_for(14.0);
                let y = pages.y - 7.0;
                pages
                    .ops
                    .extend(stroke((left, y), (PAGE_WIDTH - MARGIN, y), gray(0.6), 0.8));
                pages.y -= 14.0;
                continue;
            }
            Kind::Code => {
                let text: String = block.pieces.iter().map(|p| p.text.as_str()).collect();
                let look = look_of(&block.pieces[0], 0, &block.kind);
                for line in text.lines() {
                    let width = measure.width(look.font, line, look.size);
                    let word = Word {
                        text: line.replace('\t', "    "),
                        look,
                        width,
                        space: false,
                    };
                    lines.push(split_wide(word, avail, &mut measure));
                }
                // Each split piece of a long line gets its own row.
                lines = lines
                    .into_iter()
                    .flat_map(|l| l.into_iter().map(|w| vec![w]))
                    .collect();
            }
            Kind::Text(_) => {
                let mut current: Vec<Word> = Vec::new();
                let mut used = 0.0;
                for piece in &block.pieces {
                    let look = look_of(piece, heading, &block.kind);
                    // Words and the spaces between them, and hard breaks.
                    let mut rest = piece.text.as_str();
                    while !rest.is_empty() {
                        let first = rest.chars().next().unwrap();
                        if first == '\n' {
                            lines.push(std::mem::take(&mut current));
                            used = 0.0;
                            rest = &rest[1..];
                            continue;
                        }
                        let space = first == ' ';
                        let end = rest
                            .find(|c: char| (c == ' ') != space || c == '\n')
                            .unwrap_or(rest.len());
                        let (text, tail) = rest.split_at(end);
                        rest = tail;
                        let width = measure.width(look.font, text, look.size);
                        let word = Word {
                            text: text.to_string(),
                            look,
                            width,
                            space,
                        };
                        if space {
                            if !current.is_empty() {
                                used += word.width;
                                current.push(word);
                            }
                            continue;
                        }
                        if used + word.width > avail && !current.is_empty() {
                            while current.last().is_some_and(|w| w.space) {
                                current.pop();
                            }
                            lines.push(std::mem::take(&mut current));
                            used = 0.0;
                        }
                        for part in split_wide(word, avail, &mut measure) {
                            if used + part.width > avail && !current.is_empty() {
                                lines.push(std::mem::take(&mut current));
                                used = 0.0;
                            }
                            used += part.width;
                            current.push(part);
                        }
                    }
                }
                while current.last().is_some_and(|w| w.space) {
                    current.pop();
                }
                lines.push(current);
            }
        }

        for (index, line) in lines.iter().enumerate() {
            let size = line.iter().map(|w| w.look.size).fold(BODY_PT, f32::max);
            let height = if line.is_empty() {
                BODY_PT * 1.3
            } else {
                size * 1.3
            };
            pages.room_for(height);
            let baseline = pages.y - size * 0.95;
            if matches!(block.kind, Kind::Code) {
                pages.ops.push(Op::SetFillColor { col: gray(0.94) });
                pages.ops.push(Op::DrawRectangle {
                    rectangle: Rect {
                        x: Pt(left - 4.0),
                        y: Pt(pages.y - height),
                        width: Pt(avail + 8.0),
                        height: Pt(height),
                        mode: Some(PaintMode::Fill),
                        winding_order: None,
                    },
                });
            }
            for depth in 1..=block.quote_depth {
                let x = MARGIN + (depth - 1) as f32 * QUOTE_INDENT + 4.0;
                pages
                    .ops
                    .extend(stroke((x, pages.y), (x, pages.y - height), gray(0.65), 1.5));
            }
            // The marker hangs to the left of the first line.
            if let (0, Some(marker)) = (index, &block.marker) {
                let marker_font = pick_font(None, false, false, false);
                let width = measure.width(marker_font, marker, BODY_PT);
                let look = Look {
                    font: marker_font,
                    size: BODY_PT,
                    color: [0, 0, 0],
                    underline: false,
                    strike: false,
                };
                pages
                    .ops
                    .extend(text_ops(marker, &look, left - width - 5.0, baseline));
            }
            // Runs of words that look the same go out as one.
            let mut x = left;
            let mut run = String::new();
            let mut run_x = x;
            let mut run_look: Option<Look> = None;
            let mut run_width = 0.0;
            for word in line {
                if run_look.is_some_and(|l| l != word.look) {
                    let look = run_look.take().unwrap();
                    pages.ops.extend(styled_run(
                        &std::mem::take(&mut run),
                        &look,
                        run_x,
                        baseline,
                        run_width,
                    ));
                    run_x = x;
                    run_width = 0.0;
                }
                run_look = Some(word.look);
                run.push_str(&word.text);
                run_width += word.width;
                x += word.width;
            }
            if let Some(look) = run_look {
                pages
                    .ops
                    .extend(styled_run(&run, &look, run_x, baseline, run_width));
            }
            pages.y -= height;
        }
        pages.y -= match block.kind {
            Kind::Code => 8.0,
            Kind::Text(level) if level > 0 => 4.0,
            _ => BODY_PT * 0.5,
        };
    }
    pages.done.push(pages.ops);

    let mut doc = PdfDocument::new(title);
    let pdf_pages: Vec<PdfPage> = pages
        .done
        .into_iter()
        .map(|ops| {
            PdfPage::new(
                Mm(PAGE_WIDTH * 25.4 / 72.0),
                Mm(PAGE_HEIGHT * 25.4 / 72.0),
                ops,
            )
        })
        .collect();
    doc.with_pages(pdf_pages)
        .save(&PdfSaveOptions::default(), &mut Vec::new())
}

/// One text operation at a position, with its color and font.
fn text_ops(text: &str, look: &Look, x: f32, y: f32) -> Vec<Op> {
    vec![
        Op::StartTextSection,
        Op::SetFont {
            font: PdfFontHandle::Builtin(look.font),
            size: Pt(look.size),
        },
        Op::SetFillColor {
            col: rgb(look.color),
        },
        Op::SetTextCursor { pos: point(x, y) },
        Op::ShowText {
            items: vec![TextItem::Text(text.to_string())],
        },
        Op::EndTextSection,
    ]
}

/// A run of text, plus the line under or through it.
fn styled_run(text: &str, look: &Look, x: f32, baseline: f32, width: f32) -> Vec<Op> {
    let mut ops = text_ops(text, look, x, baseline);
    let thickness = (look.size / 20.0).max(0.5);
    if look.underline {
        ops.extend(stroke(
            (x, baseline - look.size * 0.12),
            (x + width, baseline - look.size * 0.12),
            rgb(look.color),
            thickness,
        ));
    }
    if look.strike {
        ops.extend(stroke(
            (x, baseline + look.size * 0.3),
            (x + width, baseline + look.size * 0.3),
            rgb(look.color),
            thickness,
        ));
    }
    ops
}

#[cfg(test)]
mod tests {
    use super::*;
    use printpdf::{PdfDocument, PdfParseOptions};

    fn parse(bytes: &[u8]) -> PdfDocument {
        PdfDocument::parse(bytes, &PdfParseOptions::default(), &mut Vec::new()).unwrap()
    }

    fn text_of(doc: &PdfDocument) -> String {
        doc.extract_text()
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn makes_a_valid_pdf_with_the_text_in_it() {
        let src = "# The Title\n\nHello *bold* and _italic_ and {-struck-} with é and “quotes”.\n\n- first item\n- second item\n\n> a quote\n\n```\nlet x = 1;\n```\n";
        let bytes = to_pdf(src, "My Book");
        assert!(bytes.starts_with(b"%PDF-"));
        assert!(String::from_utf8_lossy(&bytes[bytes.len() - 16..]).contains("%%EOF"));
        let doc = parse(&bytes);
        assert_eq!(doc.metadata.info.document_title, "My Book");
        assert_eq!(doc.pages.len(), 1);
        let text = text_of(&doc);
        for want in [
            "The Title",
            "Hello",
            "bold",
            "italic",
            "struck",
            "é",
            "quotes",
            "first item",
            "second item",
            "a quote",
            "let x = 1;",
        ] {
            assert!(text.contains(want), "{want:?} missing from {text:?}");
        }
    }

    fn ops_of(src: &str) -> Vec<Op> {
        parse(&to_pdf(src, "T")).pages.remove(0).ops
    }

    #[test]
    fn styling_reaches_the_pdf() {
        let ops = ops_of(
            "# Head\n\n*b* _i_ [big red]{style=\"color:#c00;font-size:14pt\"} [sans]{style=\"font-family:Arial\"} `mono` [c]{style=\"font-family:Courier\"}",
        );
        let fonts: Vec<(BuiltinFont, f32)> = ops
            .iter()
            .filter_map(|op| match op {
                Op::SetFont {
                    font: PdfFontHandle::Builtin(f),
                    size,
                } => Some((*f, size.0)),
                _ => None,
            })
            .collect();
        for want in [
            (BuiltinFont::TimesBold, 24.0),
            (BuiltinFont::TimesBold, 12.0),
            (BuiltinFont::TimesItalic, 12.0),
            (BuiltinFont::TimesRoman, 14.0),
            (BuiltinFont::Helvetica, 12.0),
            (BuiltinFont::Courier, 10.8),
            (BuiltinFont::Courier, 12.0),
        ] {
            let found = fonts
                .iter()
                .any(|&(f, size)| f == want.0 && (size - want.1).abs() < 0.01);
            assert!(found, "{want:?} not in {fonts:?}");
        }
        let reds = ops
            .iter()
            .filter(|op| matches!(op, Op::SetFillColor { col: Color::Rgb(c) } if c.r > 0.79 && c.r < 0.81 && c.g == 0.0))
            .count();
        assert_eq!(reds, 1);
    }

    #[test]
    fn underline_and_strikethrough_are_drawn_as_lines() {
        let lines = |src: &str| {
            ops_of(src)
                .iter()
                .filter(|op| matches!(op, Op::DrawLine { .. }))
                .count()
        };
        assert_eq!(lines("plain text"), 0);
        assert_eq!(lines("{-struck-}"), 1);
        assert_eq!(lines("[u]{style=\"text-decoration:underline\"}"), 1);
        // Links are underlined and blue.
        assert_eq!(lines("[a link](http://x.y)"), 1);
        assert_eq!(lines("***"), 1);
    }

    #[test]
    fn long_text_flows_onto_more_pages_and_stays_inside_the_margins() {
        let src: String = (0..150)
            .map(|i| format!("Paragraph {i} lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua.\n\n"))
            .collect();
        let doc = parse(&to_pdf(&src, "Long"));
        assert!(doc.pages.len() > 2);
        let text = text_of(&doc);
        assert!(text.contains("Paragraph 0 ") && text.contains("Paragraph 149 "));
        for page in &doc.pages {
            for op in &page.ops {
                if let Op::SetTextCursor { pos } = op {
                    assert!(
                        pos.x.0 >= MARGIN - 0.1 && pos.x.0 < PAGE_WIDTH - MARGIN,
                        "x {}",
                        pos.x.0
                    );
                    assert!(
                        pos.y.0 >= MARGIN - 0.1 && pos.y.0 <= PAGE_HEIGHT - MARGIN,
                        "y {}",
                        pos.y.0
                    );
                }
            }
        }
    }

    #[test]
    fn edge_cases_still_make_a_pdf() {
        // Nothing to write is still one blank page.
        let doc = parse(&to_pdf("", "Empty"));
        assert_eq!(doc.pages.len(), 1);
        assert!(text_of(&doc).trim().is_empty());
        // A word wider than the page is split, not lost, and characters outside Windows-1252 don't
        // break it.
        let wide = "x".repeat(300);
        let doc = parse(&to_pdf(
            &format!("{wide}\n\n```\n{wide}\n```\n\n漢字 ok"),
            "Wide",
        ));
        let text = text_of(&doc).replace(' ', "");
        assert!(text.contains(&wide));
        assert!(text.contains("ok"));
    }
}
