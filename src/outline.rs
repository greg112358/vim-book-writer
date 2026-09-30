//! The headings of a document, for the outline sidebar.

/// One heading: its level (1 to 6), its text with the emphasis marks removed, and the byte
/// offset of its line.
#[derive(Debug, Clone, PartialEq)]
pub struct Heading {
    pub level: usize,
    pub title: String,
    pub offset: usize,
}

/// The headings in `text`, in order. A heading is a line of one to six `#` and a space; lines
/// inside a fenced code block are not.
pub fn headings(text: &str) -> Vec<Heading> {
    let mut found = Vec::new();
    let mut fence: Option<&str> = None;
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_end();
        let ticks = trimmed
            .trim_start()
            .split(|c| c != '`' && c != '~')
            .next()
            .filter(|f| f.len() >= 3);
        match (fence, ticks) {
            (None, Some(open)) => fence = Some(open),
            (Some(open), Some(close))
                if close.starts_with(open.chars().next().unwrap()) && close.len() >= open.len() =>
            {
                fence = None
            }
            (None, None) => {
                let level = trimmed.bytes().take_while(|&b| b == b'#').count();
                if (1..=6).contains(&level) && trimmed[level..].starts_with(' ') {
                    let title: String = trimmed[level..]
                        .trim()
                        .chars()
                        .filter(|&c| c != '*' && c != '`')
                        .collect();
                    if !title.is_empty() {
                        found.push(Heading {
                            level,
                            title,
                            offset,
                        });
                    }
                }
            }
            _ => {}
        }
        offset += line.len();
    }
    found
}

/// The heading the cursor at `cursor` is under: the last one that starts at or before it.
pub fn current(headings: &[Heading], cursor: usize) -> Option<usize> {
    headings
        .partition_point(|h| h.offset <= cursor)
        .checked_sub(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_headings_but_not_ones_in_code_or_without_a_space() {
        let text = "# One\ntext\n## *Two* `x`\n```\n# not\n```\n#nope\n####### seven\n### Three\n";
        let found = headings(text);
        let got: Vec<(usize, &str, usize)> = found
            .iter()
            .map(|h| (h.level, h.title.as_str(), h.offset))
            .collect();
        let three = text.find("### Three").unwrap();
        assert_eq!(got, [(1, "One", 0), (2, "Two x", 11), (3, "Three", three)]);
        // The cursor is under the last heading at or before it.
        assert_eq!(current(&found, 0), Some(0));
        assert_eq!(current(&found, 30), Some(1));
        assert_eq!(current(&found, 200), Some(2));
        assert_eq!(current(&headings("text\n# A"), 1), None);
    }
}
