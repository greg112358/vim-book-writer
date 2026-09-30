use crate::djot::parse;
use crate::runs::{Format, runs, visible};
use std::ops::Range;

/// A piece of the displayed text with one look.
#[derive(Debug, Clone, PartialEq)]
pub struct Span {
    /// Where it sits in `Display::text`.
    pub range: Range<usize>,
    /// Where it came from in the source.
    pub source: Range<usize>,
    pub format: Format,
    /// Syntax the writer typed, drawn dimmed so the text stays readable.
    pub faint: bool,
}

/// What the screen shows for a document: the text, and how each stretch of it looks.
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
        Self::new(String::new(), Vec::new())
    }
}

impl Display {
    /// Normal mode: the document as it reads, with delimiters, block markers, link targets, and
    /// attribute blocks hidden. Each span keeps its source range, so the cursor can be put back
    /// in the source.
    pub fn normal(src: &str) -> Self {
        let runs = runs(src, &parse(src));
        let shown = visible(src, &runs);
        let spans = shown
            .runs
            .iter()
            .map(|(range, run)| Span {
                range: range.clone(),
                source: run.range.clone(),
                format: run.format.clone(),
                faint: false,
            })
            .collect();
        Self::new(shown.text, spans)
    }

    /// Insert mode: the source as typed, with each delimiter, block marker, and attribute block
    /// marked faint and the words in the style they will have. `**Bold Text**` is faint `**`,
    /// bold `Bold Text`, faint `**`.
    pub fn insert(src: &str) -> Self {
        let spans = runs(src, &parse(src))
            .into_iter()
            .map(|run| Span {
                range: run.range.clone(),
                source: run.range,
                format: run.format,
                faint: run.markup,
            })
            .collect();
        Self::new(src.to_string(), spans)
    }

    fn new(text: String, spans: Vec<Span>) -> Self {
        let line_starts = std::iter::once(0)
            .chain(text.match_indices('\n').map(|(i, _)| i + 1))
            .collect();
        Self {
            text,
            spans,
            line_starts,
        }
    }

    /// The displayed offset that shows source offset `source`. A source offset inside hidden
    /// syntax is shown where the next visible text is.
    pub fn to_display(&self, source: usize) -> usize {
        let i = self.spans.partition_point(|s| s.source.end <= source);
        match self.spans.get(i) {
            Some(s) if s.source.start <= source => s.range.start + (source - s.source.start),
            Some(s) => s.range.start,
            None => self.text.len(),
        }
    }

    /// The source offset that displayed offset `display` stands for.
    pub fn to_source(&self, display: usize) -> usize {
        let i = self.spans.partition_point(|s| s.range.end <= display);
        match self.spans.get(i) {
            Some(s) => s.source.start + (display - s.range.start),
            None => self.spans.last().map_or(0, |s| s.source.end),
        }
    }

    /// Moves a source offset out of hidden syntax, forward to the next visible char or, if
    /// `forward` is false or nothing follows, back to the last visible one before it.
    pub fn snap(&self, source: usize, forward: bool) -> usize {
        let i = self.spans.partition_point(|s| s.source.end <= source);
        if self.spans.get(i).is_some_and(|s| s.source.start <= source) {
            return source;
        }
        let next = self.spans.get(i).map(|s| s.source.start);
        let before = i.checked_sub(1).map(|j| {
            let s = &self.spans[j];
            let last = self.text[..s.range.end]
                .chars()
                .next_back()
                .map_or(0, char::len_utf8);
            s.source.end - last
        });
        if forward {
            next.or(before)
        } else {
            before.or(next)
        }
        .unwrap_or(source)
    }

    /// The line and byte offset within it of displayed offset `display`.
    pub fn line_and_index(&self, display: usize) -> (usize, usize) {
        let line = self.line_starts.partition_point(|&start| start <= display) - 1;
        (line, display - self.line_starts[line])
    }

    /// The displayed offset of byte `index` of line `line`.
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
    fn normal_mode_shows_the_words_with_their_looks() {
        let src = "## Title\n\n*bold* and _it_ {-x-} [red]{.note}\n";
        let display = Display::normal(src);
        assert_eq!(display.text, "Title\n\nbold and it x red\n");
        // Each span is a slice of the text, and knows where in the source it came from.
        let look = |word: &str| {
            let span = display
                .spans
                .iter()
                .find(|s| &display.text[s.range.clone()] == word)
                .unwrap();
            (span.format.clone(), &src[span.source.clone()])
        };
        let (title, source) = look("Title");
        assert_eq!((title.heading, source), (2, "Title"));
        assert!(look("bold").0.bold && !look("bold").0.italic);
        assert!(look("it").0.italic);
        assert!(look("x").0.strike);
        assert_eq!(look("red").1, "red");
        // The spans cover the whole text with no gaps.
        let mut end = 0;
        for span in &display.spans {
            assert_eq!(span.range.start, end);
            end = span.range.end;
        }
        assert_eq!(end, display.text.len());
        assert_eq!(Display::normal(""), Display::default());
        assert!(display.spans.iter().all(|s| !s.faint));
    }

    #[test]
    fn insert_mode_shows_the_source_with_the_syntax_faint() {
        let src = "## Head\n\n**Bold Text** and [red]{.note}\n";
        let display = Display::insert(src);
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
        // The words and their delimiters are bold, and the source ranges are the text's own.
        let bold: Vec<bool> = display.spans[3..6].iter().map(|s| s.format.bold).collect();
        assert_eq!(bold, [true, true, true]);
        assert!(display.spans.iter().all(|s| s.range == s.source));
        assert_eq!(Display::insert(""), Display::default());
    }

    #[test]
    fn positions_map_between_source_and_display() {
        // Source:  ## Hi\n\na **b** c\n   Display: Hi\n\na b c\n
        let src = "## Hi\n\na **b** c\n";
        let display = Display::normal(src);
        assert_eq!(display.text, "Hi\n\na b c\n");
        let b = src.find('b').unwrap();
        assert_eq!(display.to_display(b), 6);
        assert_eq!(display.to_source(6), b);
        // Every visible char round-trips.
        for (i, _) in display.text.char_indices() {
            assert_eq!(display.to_display(display.to_source(i)), i, "{i}");
        }
        // Hidden syntax is shown where the next visible text is; the end is the end.
        assert_eq!(display.to_display(0), 0);
        assert_eq!(display.to_display(src.find("**").unwrap()), 6);
        assert_eq!(display.to_display(src.len()), display.text.len());
        assert_eq!(display.to_source(display.text.len()), src.len());
        // The insert display is the identity.
        let insert = Display::insert(src);
        assert!((0..=src.len()).all(|i| insert.to_display(i) == i && insert.to_source(i) == i));
    }

    #[test]
    fn snap_moves_off_hidden_syntax_in_the_direction_of_travel() {
        let src = "a **b** c";
        let display = Display::normal(src);
        // a0 ' '1 *2 *3 b4 *5 *6 ' '7 c8
        assert_eq!(display.snap(4, true), 4);
        assert_eq!(display.snap(3, true), 4);
        assert_eq!(display.snap(3, false), 1);
        assert_eq!(display.snap(5, true), 7);
        assert_eq!(display.snap(6, false), 4);
        // With nothing further that way it goes the other way.
        let display = Display::normal("**a**");
        assert_eq!(display.snap(0, false), 2);
        assert_eq!(display.snap(4, true), 2);
        assert_eq!(Display::normal("").snap(0, true), 0);
        // Multi-byte chars: the last visible char starts at its first byte.
        assert_eq!(Display::normal("é**a**").snap(2, false), 0);
    }

    #[test]
    fn lines_and_offsets() {
        let display = Display::insert("ab\n\ncd");
        assert_eq!(display.line_and_index(0), (0, 0));
        assert_eq!(display.line_and_index(2), (0, 2));
        assert_eq!(display.line_and_index(3), (1, 0));
        assert_eq!(display.line_and_index(6), (2, 2));
        assert_eq!(display.offset(2, 1), 5);
        assert_eq!(display.offset(9, 0), 6);
    }
}
