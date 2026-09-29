use crate::rects::Rect;
use book_writing_app::editor::View;
use book_writing_app::runs::Format;
use book_writing_app::vim::Mode;
use glyphon::{Attrs, Buffer, Color, Cursor, Family, Metrics, Style, Weight};

/// Body text size in pixels, and the line height as a multiple of it.
pub const FONT_SIZE: f32 = 18.0;
pub const LINE_HEIGHT: f32 = 1.5;

pub const TEXT: Color = Color::rgb(0x22, 0x22, 0x22);
const FAINT: Color = Color::rgb(0xb4, 0xab, 0x9a);
const LINK: Color = Color::rgb(0x1a, 0x5f, 0xb4);
const QUOTE: Color = Color::rgb(0x5a, 0x5a, 0x5a);

/// How a span of text looks: family, size, weight, slant, color, and decorations. `faint` is
/// syntax shown dimmed, in a color that reads as part of the page but not part of the text.
pub fn attrs(format: &Format, faint: bool) -> Attrs<'_> {
    let scale = [1.0, 2.0, 1.6, 1.3, 1.15, 1.05, 1.0][format.heading.min(6) as usize];
    let mut size = FONT_SIZE * scale;
    // Syntax stays at the size and family of the text around it, however the span is styled.
    if let Some(pt) = format.style.size_pt.filter(|_| !faint) {
        size = pt * 96.0 / 72.0;
    }
    let mono = format.code || format.code_block;
    if mono {
        size *= 0.9;
    }
    let family = match &format.style.family {
        Some(name) if !faint => Family::Name(name),
        _ if mono => Family::Monospace,
        _ => Family::Serif,
    };
    let color = match format.style.color {
        _ if faint => FAINT,
        Some([r, g, b]) => Color::rgb(r, g, b),
        None if format.link => LINK,
        None if format.quote => QUOTE,
        None => TEXT,
    };
    let mut attrs = Attrs::new()
        .family(family)
        .color(color)
        .metrics(Metrics::new(size, size * LINE_HEIGHT));
    if format.bold || format.heading > 0 {
        attrs = attrs.weight(Weight::BOLD);
    }
    if format.italic || format.quote {
        attrs = attrs.style(Style::Italic);
    }
    // glyphon doesn't draw underlines or strikethroughs, and cosmic-text joins the decorated
    // stretches on either side of a gap, so mark the glyphs and draw the lines in `decorations`.
    if !faint {
        attrs.metadata =
            usize::from(format.style.underline || format.link) | usize::from(format.strike) << 1;
    }
    attrs
}

const UNDERLINE: usize = 1;
const STRIKE: usize = 2;

/// The lines for underlines and strikethroughs, for text laid out in `buffer` and drawn at
/// (`left`, `top`). A line runs along consecutive glyphs marked for it.
pub fn decorations(buffer: &Buffer, left: f32, top: f32) -> Vec<Rect> {
    let mut rects = Vec::new();
    for run in buffer.layout_runs() {
        for (flag, offset) in [(UNDERLINE, 0.12), (STRIKE, -0.3)] {
            let mut glyphs = run.glyphs.iter().peekable();
            while let Some(first) = glyphs.find(|g| g.metadata & flag != 0) {
                let mut end = first.x + first.w;
                while let Some(next) = glyphs.next_if(|g| g.metadata & flag != 0) {
                    end = next.x + next.w;
                }
                let thickness = (first.font_size / 18.0).max(1.0);
                let color = first.color_opt.unwrap_or(TEXT);
                rects.push(Rect {
                    x: left + first.x,
                    y: top + run.line_y + offset * first.font_size - thickness / 2.0,
                    w: end - first.x,
                    h: thickness,
                    color: [color.r(), color.g(), color.b(), color.a()],
                });
            }
        }
    }
    rects
}

const SELECTION: [u8; 4] = [0x7f, 0xae, 0xe6, 0x80];
const BLOCK: [u8; 4] = [0x2b, 0x5c, 0xb8, 0x66];
const BAR: [u8; 4] = [0x2b, 0x5c, 0xb8, 0xff];

/// The selection highlight, which goes behind the text and is pushed to `under`, and the caret,
/// which goes in front and is pushed to `over`. `origin` is where the text's top left corner is
/// drawn. Inserting shows a thin bar between chars; otherwise a block covers the char it is on.
/// Returns the top and bottom of the caret's line, measured from the top of the text.
pub fn caret_and_selection(
    buffer: &Buffer,
    view: &View<'_>,
    (left, top): (f32, f32),
    under: &mut Vec<Rect>,
    over: &mut Vec<Rect>,
) -> Option<(f32, f32)> {
    let display = view.display;
    let cursor_at = |offset: usize| {
        let (line, index) = display.line_and_index(offset);
        Cursor::new(line, index)
    };
    let selection = view
        .selection
        .as_ref()
        .map(|r| (cursor_at(r.start), cursor_at(r.end)));
    let (cursor_line, cursor_index) = display.line_and_index(view.cursor);
    // The last run of the caret's line, in case the caret sits past its last glyph.
    let mut caret: Option<(f32, f32, f32, f32)> = None;
    for run in buffer.layout_runs() {
        let (y, height) = (top + run.line_top, run.line_height);
        // `highlight` assumes the run's line is in range, so check that here.
        if let Some((start, end)) =
            selection.filter(|(a, b)| (a.line..=b.line).contains(&run.line_i))
        {
            let mut any = false;
            for (x, w) in run.highlight(start, end) {
                under.push(Rect {
                    x: left + x,
                    y,
                    w,
                    h: height,
                    color: SELECTION,
                });
                any = true;
            }
            // A selected empty line has no glyphs to highlight, so show its newline.
            if !any && run.glyphs.is_empty() && (start.line..end.line).contains(&run.line_i) {
                under.push(Rect {
                    x: left,
                    y,
                    w: FONT_SIZE * 0.5,
                    h: height,
                    color: SELECTION,
                });
            }
        }
        if run.line_i == cursor_line {
            let glyph = run
                .glyphs
                .iter()
                .find(|g| g.start <= cursor_index && cursor_index < g.end);
            match glyph {
                Some(g) => caret = Some((left + g.x, g.w, y, height)),
                None if caret.is_none()
                    || run.glyphs.last().is_some_and(|g| g.end <= cursor_index) =>
                {
                    let x = run.glyphs.last().map_or(0.0, |g| g.x + g.w);
                    caret = Some((left + x, FONT_SIZE * 0.5, y, height));
                }
                None => {}
            }
        }
    }
    if let Some((x, w, y, h)) = caret {
        over.push(match view.mode {
            Mode::Insert => Rect {
                x,
                y,
                w: 2.0,
                h,
                color: BAR,
            },
            _ => Rect {
                x,
                y,
                w,
                h,
                color: BLOCK,
            },
        });
    }
    caret.map(|(_, _, y, h)| (y - top, y - top + h))
}

/// The bottom of the last line of text in `buffer`.
pub fn text_height(buffer: &Buffer) -> f32 {
    buffer
        .layout_runs()
        .last()
        .map_or(0.0, |run| run.line_top + run.line_height)
}

/// The line and byte index in it of the text at (`x`, `y`), measured from the text's top left
/// corner. A point above or below the text is on its first or last line, and one to the right of
/// a line is at its end. With `block` it is the char under the point, as for a block cursor;
/// without, the boundary nearest the point, as for a bar.
pub fn hit(buffer: &Buffer, x: f32, y: f32, block: bool) -> Option<(usize, usize)> {
    let run = buffer
        .layout_runs()
        .find(|run| y < run.line_top + run.line_height)
        .or_else(|| buffer.layout_runs().last())?;
    let index = run.glyphs.iter().find(|g| x < g.x + g.w).map_or_else(
        || run.glyphs.last().map_or(0, |g| g.end),
        |g| {
            if block || x < g.x + g.w / 2.0 {
                g.start
            } else {
                g.end
            }
        },
    );
    Some((run.line_i, index))
}
