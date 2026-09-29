use crate::rects::Rect;
use book_writing_app::runs::Format;
use glyphon::{Attrs, Buffer, Color, Family, Metrics, Style, Weight};

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
