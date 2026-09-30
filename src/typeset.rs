use glyphon::{Attrs, Buffer, Color, Cursor, Family, Metrics, Style, Weight};
use vim_book_writer::diff::Change;
use vim_book_writer::editor::View;
use vim_book_writer::runs::Format;
use vim_book_writer::ui::{Rect, Theme};
use vim_book_writer::vim::Mode;

pub use vim_book_writer::ui::{LINE_HEIGHT, Reading};

fn color([r, g, b, a]: [u8; 4]) -> Color {
    Color::rgba(r, g, b, a)
}

pub fn text_color(theme: &Theme) -> Color {
    color(theme.text)
}

/// How a span of text looks: family, size, weight, slant, color, and decorations. `faint` is
/// syntax shown dimmed, in a color that reads as part of the page but not part of the text.
/// The family and size are how the text is being read, not something the document says.
pub fn attrs<'a>(format: &Format, faint: bool, theme: &Theme, reading: &'a Reading) -> Attrs<'a> {
    let mut attrs = look(format, faint, theme, reading);
    // Superscripts and subscripts are drawn again, raised or lowered, by `Gpu`; here they only
    // hold their room in the line, so they aren't drawn.
    if !faint && (format.superscript || format.subscript) {
        attrs = attrs.color(Color::rgba(0, 0, 0, 0));
    }
    attrs
}

/// The size of the text of `format` before it is made smaller for a superscript or subscript.
pub fn base_size(format: &Format, reading: &Reading) -> f32 {
    let scale = [1.0, 2.0, 1.6, 1.3, 1.15, 1.05, 1.0][format.heading.min(6) as usize];
    let size = reading.size() * scale;
    if format.code || format.code_block {
        size * 0.9
    } else {
        size
    }
}

/// How a superscript or subscript looks when it is drawn: the smaller size, and `raise`, how far
/// above the line it goes, which is negative below it.
pub fn script_attrs<'a>(format: &Format, theme: &Theme, reading: &'a Reading) -> (Attrs<'a>, f32) {
    let base = base_size(format, reading);
    let raise = if format.superscript {
        0.36 * base
    } else {
        -0.14 * base
    };
    (look(format, false, theme, reading), raise)
}

fn look<'a>(format: &Format, faint: bool, theme: &Theme, reading: &'a Reading) -> Attrs<'a> {
    let mut size = base_size(format, reading);
    if !faint && (format.superscript || format.subscript) {
        size *= 0.65;
    }
    let mono = format.code || format.code_block;
    let family = if mono {
        Family::Monospace
    } else {
        family(&reading.family)
    };
    let color = match () {
        _ if faint => color(theme.faint),
        _ if format.link => color(theme.link),
        _ if format.quote => color(theme.quote),
        _ => color(theme.text),
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
        attrs.metadata = usize::from(format.link) | usize::from(format.strike) << 1;
    }
    attrs
}

const UNDERLINE: usize = 1;
const STRIKE: usize = 2;

/// The font family for the name of a reading font. The three generic names mean whichever font
/// of that kind is installed.
fn family(name: &str) -> Family<'_> {
    match name.to_ascii_lowercase().as_str() {
        "serif" => Family::Serif,
        "sans" | "sans-serif" => Family::SansSerif,
        "mono" | "monospace" => Family::Monospace,
        _ => Family::Name(name),
    }
}

/// The lines for underlines and strikethroughs, for text laid out in `buffer` and drawn at
/// (`left`, `top`). A line runs along consecutive glyphs marked for it.
pub fn decorations(buffer: &Buffer, left: f32, top: f32, theme: &Theme) -> Vec<Rect> {
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
                let color = first.color_opt.unwrap_or_else(|| text_color(theme));
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
    theme: &Theme,
    reading: &Reading,
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
        // Searched-for text, behind the selection. Matches are in order, so find this line's.
        let (line_lo, line_hi) = (
            display.offset(run.line_i, 0),
            display.offset(run.line_i + 1, 0),
        );
        let first = view.found.partition_point(|m| m.end <= line_lo);
        for m in view.found[first..].iter().take_while(|m| m.start < line_hi) {
            let at =
                |offset: usize| Cursor::new(run.line_i, offset.clamp(line_lo, line_hi) - line_lo);
            let color = if m.contains(&view.cursor) {
                theme.found_here
            } else {
                theme.found
            };
            for (x, w) in run.highlight(at(m.start), at(m.end)) {
                under.push(Rect {
                    x: left + x,
                    y,
                    w,
                    h: height,
                    color,
                });
            }
        }
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
                    color: theme.selection,
                });
                any = true;
            }
            // A selected empty line has no glyphs to highlight, so show its newline.
            if !any && run.glyphs.is_empty() && (start.line..end.line).contains(&run.line_i) {
                under.push(Rect {
                    x: left,
                    y,
                    w: reading.size() * 0.5,
                    h: height,
                    color: theme.selection,
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
                    caret = Some((left + x, reading.size() * 0.5, y, height));
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
                color: theme.caret,
            },
            _ => Rect {
                x,
                y,
                w,
                h,
                color: theme.block,
            },
        });
    }
    caret.map(|(_, _, y, h)| (y - top, y - top + h))
}

/// The bars in the margin beside lines that differ from the last commit, for text laid out in
/// `buffer` and drawn at (`left`, `top`). A wrapped line has its bar on every row.
pub fn gutter(
    buffer: &Buffer,
    view: &View<'_>,
    (left, top): (f32, f32),
    theme: &Theme,
) -> Vec<Rect> {
    let mut bars = Vec::new();
    for run in buffer.layout_runs() {
        let Ok(at) = view
            .changes
            .binary_search_by_key(&run.line_i, |&(line, _)| line)
        else {
            continue;
        };
        let change = view.changes[at].1;
        let (y, h) = (top + run.line_top, run.line_height);
        // Lines taken out are a notch at the top of the line that follows.
        let (w, h) = if change == Change::Removed {
            (10.0, 2.0)
        } else {
            (3.0, h)
        };
        bars.push(Rect {
            x: left - 14.0,
            y,
            w,
            h,
            color: theme.gutter(change),
        });
    }
    bars
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
