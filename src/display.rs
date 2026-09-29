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
#[derive(Debug, Default, PartialEq)]
pub struct Display {
    pub text: String,
    /// In order, without gaps, tiling `text`.
    pub spans: Vec<Span>,
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
        Self {
            text: shown.text,
            spans,
        }
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
        Self {
            text: src.to_string(),
            spans,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_mode_shows_the_words_with_their_looks() {
        let src = "## Title\n\n*bold* and _it_ {-x-} [red]{style=\"color:#c00\"}\n";
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
        assert_eq!(look("red").0.style.color, Some([0xcc, 0, 0]));
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
        let src = "## Head\n\n**Bold Text** and [red]{style=\"color:#c00\"}\n";
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
                ("]{style=\"color:#c00\"}", true),
                ("\n", false),
            ]
        );
        // The words and their delimiters are bold, and the source ranges are the text's own.
        let bold: Vec<bool> = display.spans[3..6].iter().map(|s| s.format.bold).collect();
        assert_eq!(bold, [true, true, true]);
        assert!(display.spans.iter().all(|s| s.range == s.source));
        assert_eq!(Display::insert(""), Display::default());
    }
}
