use crate::djot::parse;
use crate::runs::{Format, runs};
use std::ops::Range;

/// A stretch of the text with one look.
#[derive(Debug, Clone, PartialEq)]
pub struct Span {
    pub range: Range<usize>,
    pub format: Format,
    /// Syntax the writer typed, drawn dimmed so the text stays readable.
    pub faint: bool,
}

/// What the screen shows for a document: the text as typed, and how each stretch of it looks.
/// The text is the source itself, so an offset in one is an offset in the other.
#[derive(Debug, PartialEq)]
pub struct Display {
    pub text: String,
    /// In order, without gaps, tiling `text`.
    pub spans: Vec<Span>,
    /// Where each line of `text` starts.
    line_starts: Vec<usize>,
}

impl Default for Display {
    fn default() -> Self {
        Self::new("")
    }
}

impl Display {
    /// `src` with each delimiter, block marker, and attribute block marked faint and the words in
    /// the style they will have: `**Bold Text**` is faint `**`, bold `Bold Text`, faint `**`.
    pub fn new(src: &str) -> Self {
        let spans = runs(src, &parse(src))
            .into_iter()
            .map(|run| Span {
                range: run.range,
                format: run.format,
                faint: run.markup,
            })
            .collect();
        let line_starts = std::iter::once(0)
            .chain(src.match_indices('\n').map(|(i, _)| i + 1))
            .collect();
        Self {
            text: src.to_string(),
            spans,
            line_starts,
        }
    }

    pub fn line_count(&self) -> usize {
        self.line_starts.len()
    }

    /// The line and byte offset within it of `offset`.
    pub fn line_and_index(&self, offset: usize) -> (usize, usize) {
        let line = self.line_starts.partition_point(|&start| start <= offset) - 1;
        (line, offset - self.line_starts[line])
    }

    /// The offset of byte `index` of line `line`, or the end of the text for a line past the last.
    pub fn offset(&self, line: usize, index: usize) -> usize {
        self.line_starts
            .get(line)
            .map_or(self.text.len(), |start| start + index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_source_is_shown_with_the_syntax_faint() {
        let src = "## Head\n\n**Bold Text** and [red]{.note}\n";
        let display = Display::new(src);
        assert_eq!(display.text, src);
        // (text, faint) for each span; neighbours that look alike were merged by `runs`.
        let shown: Vec<(&str, bool)> = display
            .spans
            .iter()
            .map(|s| (&display.text[s.range.clone()], s.faint))
            .collect();
        assert_eq!(
            shown,
            [
                ("## ", true),
                ("Head", false),
                ("\n\n", false),
                ("**", true),
                ("Bold Text", false),
                ("**", true),
                (" and ", false),
                ("[", true),
                ("red", false),
                ("]{.note}", true),
                ("\n", false),
            ]
        );
        // The words and their delimiters are bold.
        let bold: Vec<bool> = display.spans[3..6].iter().map(|s| s.format.bold).collect();
        assert_eq!(bold, [true, true, true]);
        assert_eq!(Display::new(""), Display::default());
    }

    #[test]
    fn lines_and_offsets() {
        let display = Display::new("ab\n\ncd");
        assert_eq!(display.line_count(), 3);
        assert_eq!(display.line_and_index(0), (0, 0));
        assert_eq!(display.line_and_index(2), (0, 2));
        assert_eq!(display.line_and_index(3), (1, 0));
        assert_eq!(display.line_and_index(6), (2, 2));
        assert_eq!(display.offset(2, 1), 5);
        assert_eq!(display.offset(9, 0), 6);
    }
}
