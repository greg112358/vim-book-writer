use jotdown::{Container, Event, LinkType, ListKind, Parser};

/// What encloses the block being written, for spacing: list items in a tight list have no blank
/// lines between them.
enum Ctx {
    List { tight: bool, next: Option<u64> },
    Item { tight: bool },
    Quote,
}

/// Text put before each line inside a list item or blockquote. `first` starts the container's
/// first line, and `rest` every later one.
struct Prefix {
    first: String,
    rest: String,
    used: bool,
}

#[derive(Default)]
struct Writer {
    out: String,
    prefixes: Vec<Prefix>,
    ctxs: Vec<Ctx>,
    line_start: bool,
    /// True until the first block of a document, list, item, or quote starts, which needs no
    /// blank line before it.
    first_in_container: bool,
    /// The text of a code block or inline code being collected, since its fence depends on it.
    code: Option<String>,
    /// While positive, text is dropped: an autolink writes its URL instead of its text.
    skip_text: u32,
    /// Where the current line's own text starts in `out`, after any prefixes.
    content_start: usize,
}

/// Whole lines and inline text need the prefixes of the containers they sit in.
impl Writer {
    fn indent(&mut self) {
        if !self.line_start {
            return;
        }
        for p in &mut self.prefixes {
            self.out.push_str(if p.used { &p.rest } else { &p.first });
            p.used = true;
        }
        self.line_start = false;
        self.content_start = self.out.len();
    }

    /// Writes `s` as it is, indenting each line that has text.
    fn text(&mut self, s: &str) {
        for (i, line) in s.split('\n').enumerate() {
            if i > 0 {
                self.out.push('\n');
                self.line_start = true;
            }
            if !line.is_empty() {
                self.indent();
                self.out.push_str(line);
            }
        }
    }

    fn end_line(&mut self) {
        if !self.line_start && !self.out.is_empty() {
            self.out.push('\n');
            self.line_start = true;
        }
    }

    /// A blank line, which inside a quote still carries the `>`.
    fn blank_line(&mut self) {
        self.end_line();
        let prefix: String = self
            .prefixes
            .iter()
            .map(|p| if p.used { p.rest.as_str() } else { "" })
            .collect();
        self.out.push_str(prefix.trim_end());
        self.out.push('\n');
    }

    fn block_start(&mut self) {
        let tight = matches!(
            self.ctxs.last(),
            Some(Ctx::List { tight: true, .. } | Ctx::Item { tight: true })
        );
        if !self.first_in_container && !self.out.is_empty() && !tight {
            self.blank_line();
        }
        self.end_line();
        self.first_in_container = false;
    }

    fn push_container(&mut self, ctx: Ctx, first: &str, rest: &str) {
        self.ctxs.push(ctx);
        self.prefixes.push(Prefix {
            first: first.into(),
            rest: rest.into(),
            used: false,
        });
        self.first_in_container = true;
    }
}

/// Backslash-escapes what Markdown would read as syntax, so the text comes out as written.
/// `so_far` is what the line already holds; the block markers `#`, `>`, `-`, `+`, and `1.` only
/// count at the start of one, and djot can split `1.` across two pieces of text.
fn escape(s: &str, so_far: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let digits = s.bytes().take_while(u8::is_ascii_digit).count();
    let numbered_at = (so_far.bytes().all(|b| b.is_ascii_digit())
        && so_far.len() + digits > 0
        && matches!(s.as_bytes().get(digits), Some(b'.' | b')'))
        && s[digits + 1..]
            .chars()
            .next()
            .is_none_or(char::is_whitespace))
    .then_some(digits);
    for (i, c) in s.char_indices() {
        let starts_line = so_far.is_empty() && i == 0;
        let next_is_space = s[i + c.len_utf8()..]
            .chars()
            .next()
            .is_none_or(char::is_whitespace);
        if matches!(c, '\\' | '`' | '*' | '_' | '[' | ']' | '<' | '~')
            || (starts_line && matches!(c, '#' | '>'))
            || (starts_line && matches!(c, '-' | '+') && next_is_space)
            || numbered_at == Some(i)
        {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Converts djot to Markdown. It is lossy: color, size, font, and underline have no CommonMark
/// form, so the styled text comes out plain. Everything else in the v1 scope maps across,
/// strikethrough as `~~`. Djot's typographic quotes, dashes, and ellipsis come out as the
/// characters they stand for.
pub fn to_markdown(src: &str) -> String {
    let mut w = Writer {
        line_start: true,
        first_in_container: true,
        ..Writer::default()
    };
    let mut code_language = String::new();
    for event in Parser::new(src) {
        match event {
            Event::Start(container, _) => match container {
                Container::Paragraph => w.block_start(),
                Container::Heading { level, .. } => {
                    w.block_start();
                    w.text(&format!("{} ", "#".repeat(level as usize)));
                }
                Container::Blockquote => {
                    w.block_start();
                    w.push_container(Ctx::Quote, "> ", "> ");
                }
                Container::List { kind, tight } => {
                    w.block_start();
                    w.ctxs.push(Ctx::List {
                        tight,
                        next: match kind {
                            ListKind::Ordered { start, .. } => Some(start),
                            _ => None,
                        },
                    });
                    w.first_in_container = true;
                }
                Container::ListItem | Container::TaskListItem { .. } => {
                    w.block_start();
                    let tight = matches!(w.ctxs.last(), Some(Ctx::List { tight: true, .. }));
                    let mut marker = match w.ctxs.last_mut() {
                        Some(Ctx::List { next: Some(n), .. }) => {
                            *n += 1;
                            format!("{}. ", *n - 1)
                        }
                        _ => "- ".to_string(),
                    };
                    if let Container::TaskListItem { checked } = container {
                        marker.push_str(if checked { "[x] " } else { "[ ] " });
                    }
                    let indent = " ".repeat(marker.len());
                    w.push_container(Ctx::Item { tight }, &marker, &indent);
                }
                Container::CodeBlock { language } => {
                    w.block_start();
                    code_language = language.to_string();
                    w.code = Some(String::new());
                }
                Container::Verbatim => w.code = Some(String::new()),
                Container::Strong => w.text("**"),
                Container::Emphasis => w.text("*"),
                Container::Delete => w.text("~~"),
                Container::Link(url, LinkType::AutoLink | LinkType::Email) => {
                    w.text(&format!("<{url}>"));
                    w.skip_text += 1;
                }
                Container::Link(..) => w.text("["),
                Container::Image(..) => w.text("!["),
                _ => {}
            },
            Event::End(container) => match container {
                Container::Paragraph | Container::Heading { .. } => w.end_line(),
                Container::Blockquote | Container::List { .. } => {
                    w.ctxs.pop();
                    if matches!(container, Container::Blockquote) {
                        w.prefixes.pop();
                    }
                    w.first_in_container = false;
                }
                Container::ListItem | Container::TaskListItem { .. } => {
                    // An item with no content still needs its marker.
                    if w.prefixes.last().is_some_and(|p| !p.used) {
                        w.indent();
                        w.out.truncate(w.out.trim_end().len());
                        w.line_start = false;
                        w.end_line();
                    }
                    w.ctxs.pop();
                    w.prefixes.pop();
                    w.first_in_container = false;
                }
                Container::CodeBlock { .. } => {
                    let code = w.code.take().unwrap_or_default();
                    let longest = code.split(|c| c != '`').map(str::len).max().unwrap_or(0);
                    let fence = "`".repeat(longest.max(2) + 1);
                    w.text(&format!("{fence}{code_language}"));
                    w.end_line();
                    for line in code.lines() {
                        if line.is_empty() {
                            w.blank_line();
                        } else {
                            w.indent();
                            w.out.push_str(line);
                            w.out.push('\n');
                            w.line_start = true;
                        }
                    }
                    w.text(&fence);
                    w.end_line();
                }
                Container::Verbatim => {
                    let code = w.code.take().unwrap_or_default();
                    let longest = code.split(|c| c != '`').map(str::len).max().unwrap_or(0);
                    let fence = "`".repeat(longest + 1);
                    let pad = if code.starts_with('`') || code.ends_with('`') {
                        " "
                    } else {
                        ""
                    };
                    w.text(&format!("{fence}{pad}{code}{pad}{fence}"));
                }
                Container::Strong => w.text("**"),
                Container::Emphasis => w.text("*"),
                Container::Delete => w.text("~~"),
                Container::Link(_, LinkType::AutoLink | LinkType::Email) => w.skip_text -= 1,
                Container::Link(url, _) | Container::Image(url, _) => {
                    let url = if url.contains([' ', '(', ')']) {
                        format!("<{url}>")
                    } else {
                        url.to_string()
                    };
                    w.text(&format!("]({url})"));
                }
                _ => {}
            },
            Event::Str(s) => match &mut w.code {
                Some(code) => code.push_str(&s),
                None if w.skip_text > 0 => {}
                None => {
                    let so_far = if w.line_start {
                        ""
                    } else {
                        &w.out[w.content_start..]
                    };
                    let escaped = escape(&s, so_far);
                    w.text(&escaped);
                }
            },
            Event::Softbreak => w.text("\n"),
            Event::Hardbreak => w.text("\\\n"),
            Event::NonBreakingSpace => w.text("\u{a0}"),
            Event::Ellipsis => w.text("…"),
            Event::EnDash => w.text("–"),
            Event::EmDash => w.text("—"),
            Event::LeftSingleQuote => w.text("‘"),
            Event::RightSingleQuote => w.text("’"),
            Event::LeftDoubleQuote => w.text("“"),
            Event::RightDoubleQuote => w.text("”"),
            Event::Symbol(name) => w.text(&format!(":{name}:")),
            Event::FootnoteReference(label) => w.text(&format!("[^{label}]")),
            Event::ThematicBreak(_) => {
                w.block_start();
                w.text("---");
                w.end_line();
            }
            Event::Blankline | Event::Escape | Event::Attributes(_) => {}
        }
    }
    let end = w.out.trim_end().len();
    w.out.truncate(end);
    if !w.out.is_empty() {
        w.out.push('\n');
    }
    w.out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_size_font_and_underline_are_dropped_and_the_text_is_kept() {
        let src = r##"[red]{style="color:#c00"} and [big]{style="font-size:24pt"} and [serif]{style="font-family:Georgia"} and [under]{style="text-decoration:underline"} and [all]{style="color:#00f;font-size:9pt;font-family:'Fira Code';text-decoration:underline"}."##;
        assert_eq!(
            to_markdown(src),
            "red and big and serif and under and all.\n"
        );
        // Styled text inside emphasis keeps the emphasis.
        assert_eq!(
            to_markdown(r#"*[bold red]{style="color:#c00"}* and _[x]{style="font-size:9pt"} y_"#),
            "**bold red** and *x y*\n"
        );
    }

    #[test]
    fn inline_formatting_links_and_code() {
        assert_eq!(
            to_markdown("*bold* _it_ {-gone-} `code` [t](http://x.y) <http://a.b> ![alt](i.png)"),
            "**bold** *it* ~~gone~~ `code` [t](http://x.y) <http://a.b> ![alt](i.png)\n"
        );
        assert_eq!(
            to_markdown("``a ` b`` and [t](a b)"),
            "``a ` b`` and [t](<a b>)\n"
        );
        assert_eq!(to_markdown("`` `x` ``"), "`` `x` ``\n");
        // Typographic characters come out as themselves.
        assert_eq!(
            to_markdown(r#""hi" it's -- ok --- so..."#),
            "“hi” it’s – ok — so…\n"
        );
    }

    #[test]
    fn blocks() {
        let src = "## Title\n\nOne\ntwo\n\n***\n\n> quote a\n> more\n>\n> quote b\n\n```rust\nlet x = 1;\n\nlet y = 2;\n```\n";
        assert_eq!(
            to_markdown(src),
            "## Title\n\nOne\ntwo\n\n---\n\n> quote a\n> more\n>\n> quote b\n\n```rust\nlet x = 1;\n\nlet y = 2;\n```\n"
        );
        // Fences grow past backticks in the code.
        assert_eq!(to_markdown("````\n```\n````\n"), "````\n```\n````\n");
        assert_eq!(
            to_markdown("> ```\n> a\n>\n> b\n> ```\n"),
            "> ```\n> a\n>\n> b\n> ```\n"
        );
        assert_eq!(to_markdown(""), "");
        assert_eq!(to_markdown("\n\n"), "");
    }

    #[test]
    fn lists() {
        assert_eq!(
            to_markdown("- a\n- b\n\n  - c\n  - d\n- e\n"),
            "- a\n- b\n  - c\n  - d\n- e\n"
        );
        assert_eq!(to_markdown("1. x\n2. y\n"), "1. x\n2. y\n");
        assert_eq!(to_markdown("3. x\n4. y\n"), "3. x\n4. y\n");
        // Loose lists keep their blank lines, and items can hold several paragraphs and quotes.
        assert_eq!(to_markdown("- a\n\n- b\n"), "- a\n\n- b\n");
        assert_eq!(
            to_markdown("- a\n\n  more\n\n- b\n"),
            "- a\n\n  more\n\n- b\n"
        );
        assert_eq!(to_markdown("- > q\n"), "- > q\n");
        assert_eq!(to_markdown("> - a\n> - b\n"), "> - a\n> - b\n");
        assert_eq!(
            to_markdown("- [ ] todo\n- [x] done\n"),
            "- [ ] todo\n- [x] done\n"
        );
        // A list right after a paragraph is set off from it.
        assert_eq!(to_markdown("text\n\n- a\n"), "text\n\n- a\n");
    }

    #[test]
    fn text_that_looks_like_markdown_is_escaped() {
        assert_eq!(
            to_markdown(r"\*not\* \_bold\_ a\`b \[x\] 1 \< 2 \~"),
            r"\*not\* \_bold\_ a\`b \[x\] 1 \< 2 \~".to_string() + "\n"
        );
        assert_eq!(to_markdown(r"\# not a heading"), "\\# not a heading\n");
        assert_eq!(to_markdown(r"\> not a quote"), "\\> not a quote\n");
        assert_eq!(to_markdown(r"\- not a list"), "\\- not a list\n");
        assert_eq!(to_markdown(r"1\. not a list"), "1\\. not a list\n");
        // Only at the start of a line.
        assert_eq!(to_markdown("a # b - c 1. d"), "a # b - c 1. d\n");
        assert_eq!(to_markdown("*x*# y"), "**x**# y\n");
        assert_eq!(to_markdown("é\\\nü"), "é\\\nü\n");
    }
}
