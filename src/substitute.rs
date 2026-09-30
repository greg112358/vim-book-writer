//! The `:s` command: reading it as it is typed, and working out what it would do.

use crate::motion::{line_end, line_start, nth_line_start};
use std::ops::Range;

/// A `:s` command, whole or partly typed.
#[derive(Debug, Clone, PartialEq)]
pub struct Substitute {
    /// The lines it covers, as typed: `%`, `3,5`, `'<,'>`, or nothing for the cursor's line.
    pub range: String,
    pub pattern: String,
    /// `None` until the delimiter after the pattern is typed.
    pub replacement: Option<String>,
    /// `g`: every match in a line, not just the first.
    pub global: bool,
}

/// Reads `:s/pattern/replacement/flags` (without the colon), with any of the usual range
/// prefixes and any punctuation as the delimiter. Patterns are plain text. `\/` (the delimiter)
/// and `\\` stand for themselves, and `\n` and `\t` in the replacement for a newline and a tab.
/// `None` if it isn't a `:s` command or has no pattern yet.
pub fn parse(cmd: &str) -> Option<Substitute> {
    let cmd = cmd.trim_start();
    let at = cmd.find(|c: char| {
        !(c.is_ascii_digit() || matches!(c, '%' | '.' | '$' | ',' | '\'' | '<' | '>'))
    })?;
    let (range, rest) = cmd.split_at(at);
    let rest = rest.strip_prefix('s')?;
    let delim = rest.chars().next()?;
    if delim.is_alphanumeric() || delim.is_whitespace() || matches!(delim, '\\' | '"' | '|') {
        return None;
    }
    // Split at the delimiters that aren't escaped: pattern, replacement, flags.
    let mut parts = vec![String::new()];
    let mut chars = rest[delim.len_utf8()..].chars();
    while let Some(c) = chars.next() {
        let last = parts.len() - 1;
        if c == '\\' {
            parts[last].push(c);
            parts[last].extend(chars.next());
        } else if c == delim && parts.len() < 3 {
            parts.push(String::new());
        } else {
            parts[last].push(c);
        }
    }
    let unescape = |text: &str, replacement: bool| {
        let mut out = String::new();
        let mut chars = text.chars();
        while let Some(c) = chars.next() {
            if c != '\\' {
                out.push(c);
                continue;
            }
            match chars.next() {
                Some(n) if n == delim || n == '\\' => out.push(n),
                Some('n') if replacement => out.push('\n'),
                Some('t') if replacement => out.push('\t'),
                Some(n) => {
                    out.push('\\');
                    out.push(n);
                }
                None => out.push('\\'),
            }
        }
        out
    };
    let pattern = unescape(&parts[0], false);
    (!pattern.is_empty()).then(|| Substitute {
        range: range.to_string(),
        pattern,
        replacement: parts.get(1).map(|r| unescape(r, true)),
        global: parts.get(2).is_some_and(|flags| flags.contains('g')),
    })
}

/// The whole lines a range covers, as bytes from the start of the first to the end of the last
/// (before its newline). `cursor` is where the cursor is and `visual` what was selected when
/// the command line was opened. `None` for a range that can't be read.
pub fn region(
    text: &str,
    cursor: usize,
    range: &str,
    visual: Option<&Range<usize>>,
) -> Option<Range<usize>> {
    if range.is_empty() {
        return Some(line_start(text, cursor)..line_end(text, cursor));
    }
    if range.contains('<') || range.contains('>') {
        let v = visual?;
        return Some(
            line_start(text, v.start)..line_end(text, v.end.saturating_sub(1).max(v.start)),
        );
    }
    let current = text[..cursor].matches('\n').count();
    let last = usize::MAX;
    let line = |part: &str| match part {
        "." => Some(current),
        "$" => Some(last),
        n => n.parse::<usize>().ok().map(|n| n.saturating_sub(1)),
    };
    let (from, to) = if range == "%" {
        (0, last)
    } else {
        let (a, b) = range.split_once(',').unwrap_or((range, range));
        (line(a)?, line(b)?)
    };
    let (from, to) = (from.min(to), from.max(to));
    Some(line_start(text, nth_line_start(text, from))..line_end(text, nth_line_start(text, to)))
}

/// What running a substitution would make.
#[derive(Debug, PartialEq)]
pub struct Substituted {
    /// The whole text afterwards.
    pub text: String,
    /// Each match in the old text and what replaced it in the new.
    pub changes: Vec<(Range<usize>, Range<usize>)>,
    /// How many lines changed.
    pub lines: usize,
}

impl Substituted {
    /// Where a cursor at `cursor` in the old text is in the new.
    pub fn map_cursor(&self, cursor: usize) -> usize {
        let mut shift = 0isize;
        for (old, new) in &self.changes {
            if cursor >= old.end {
                shift = new.end as isize - old.end as isize;
            } else if cursor > old.start {
                return new.start;
            } else {
                break;
            }
        }
        (cursor as isize + shift) as usize
    }
}

/// Replaces `pattern` with `replacement` in the lines of `text` inside `region`: the first match
/// in each line, or all of them with `global`.
pub fn substitute(
    text: &str,
    region: Range<usize>,
    pattern: &str,
    replacement: &str,
    global: bool,
) -> Substituted {
    let mut out = String::with_capacity(text.len());
    out.push_str(&text[..region.start]);
    let (mut changes, mut lines, mut at) = (Vec::new(), 0, region.start);
    for line in text[region.clone()].split_inclusive('\n') {
        let mut done = 0;
        for (i, _) in line.match_indices(pattern) {
            out.push_str(&line[done..i]);
            let start = out.len();
            out.push_str(replacement);
            changes.push(((at + i)..(at + i + pattern.len()), start..out.len()));
            done = i + pattern.len();
            if !global {
                break;
            }
        }
        out.push_str(&line[done..]);
        lines += usize::from(done > 0);
        at += line.len();
    }
    out.push_str(&text[region.end..]);
    Substituted {
        text: out,
        changes,
        lines,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sub(
        range: &str,
        pattern: &str,
        replacement: Option<&str>,
        global: bool,
    ) -> Option<Substitute> {
        Some(Substitute {
            range: range.into(),
            pattern: pattern.into(),
            replacement: replacement.map(Into::into),
            global,
        })
    }

    #[test]
    fn reads_the_command_as_it_is_typed() {
        for (typed, want) in [
            ("s/a/b/g", sub("", "a", Some("b"), true)),
            ("%s/foo/bar/", sub("%", "foo", Some("bar"), false)),
            ("%s/foo/bar", sub("%", "foo", Some("bar"), false)),
            ("%s/foo/", sub("%", "foo", Some(""), false)),
            ("%s/foo", sub("%", "foo", None, false)),
            ("3,5s#a/b#c#g", sub("3,5", "a/b", Some("c"), true)),
            ("'<,'>s/x/y/", sub("'<,'>", "x", Some("y"), false)),
            (r"s/a\/b/c\/d\\/", sub("", "a/b", Some(r"c/d\"), false)),
            (r"s/a/x\ny\tz/", sub("", "a", Some("x\ny\tz"), false)),
            ("%s/ü/ue/g", sub("%", "ü", Some("ue"), true)),
            // Not substitutions, or nothing to look for yet.
            ("%s/", None),
            ("%s", None),
            ("set", None),
            ("w", None),
            ("sa/b/", None),
            ("42", None),
        ] {
            assert_eq!(parse(typed), want, "{typed:?}");
        }
    }

    #[test]
    fn ranges_cover_whole_lines() {
        let text = "one\ntwo\nthree\nfour";
        let visual = 5..9; // "wo\nt", inside lines 2 and 3
        for (range, cursor, want) in [
            ("", 5, Some(4..7)),
            ("%", 5, Some(0..18)),
            ("2", 0, Some(4..7)),
            ("2,3", 0, Some(4..13)),
            ("3,2", 0, Some(4..13)),
            (".,$", 5, Some(4..18)),
            ("2,99", 0, Some(4..18)),
            ("'<,'>", 0, Some(4..13)),
            ("x", 0, None),
        ] {
            assert_eq!(
                region(text, cursor, range, Some(&visual)),
                want,
                "{range:?}"
            );
        }
        assert_eq!(region(text, 0, "'<,'>", None), None);
        // A final newline ends the last line rather than starting another.
        assert_eq!(region("a\nb\n", 0, "%", None), Some(0..3));
    }

    #[test]
    fn replaces_the_first_or_every_match_per_line_and_maps_the_cursor() {
        let text = "aXa Xa\nno\nXX";
        let all = substitute(text, 0..text.len(), "X", "--", true);
        assert_eq!(all.text, "a--a --a\nno\n----");
        assert_eq!((all.changes.len(), all.lines), (4, 2));
        let first = substitute(text, 0..text.len(), "X", "--", false);
        assert_eq!(first.text, "a--a Xa\nno\n--X");
        // Only the lines in the region change.
        let one = substitute(text, 0..6, "X", "", true);
        assert_eq!(one.text, "aa a\nno\nXX");
        // A cursor before a match, on one, and after one keeps its place in the text around.
        assert_eq!(all.map_cursor(0), 0);
        assert_eq!(all.map_cursor(1), 1);
        assert_eq!(all.map_cursor(2), 3);
        assert_eq!(all.map_cursor(text.len()), all.text.len());
        // Nothing found changes nothing.
        let none = substitute(text, 0..text.len(), "Q", "z", true);
        assert!(none.changes.is_empty() && none.text == text);
    }
}
