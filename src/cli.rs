use crate::vim::{Key, Mode};
use std::path::PathBuf;

const USAGE: &str =
    "usage: book-writing-app [FILE] [--file FILE] [--screenshot PNG [--mode MODE] [--size WxH]]

  FILE, --file FILE   the djot document to open
  --screenshot PNG    draw one frame offscreen to PNG and exit
  --mode MODE         the mode to show: normal, insert, visual, or visual-line
  --size WxH          the size of the screenshot in pixels (default 1000x700)
  --tree              show the file tree
  --keys KEYS         keys to press before the screenshot, after the mode is set: text as typed,
                      plus <Esc> <CR> <BS> <Del> <Left> <Right> <Up> <Down> <C-x> and <lt>";

/// What the command line asks for.
#[derive(Debug, Clone, PartialEq)]
pub struct Args {
    pub file: Option<PathBuf>,
    /// If set, render one frame to this PNG instead of opening a window.
    pub screenshot: Option<PathBuf>,
    pub mode: Mode,
    pub size: (u32, u32),
    /// Keys to press before a screenshot, once in `mode`.
    pub keys: Vec<Key>,
    /// Show the file tree on the right.
    pub tree: bool,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            file: None,
            screenshot: None,
            mode: Mode::Normal,
            size: (1000, 700),
            keys: Vec::new(),
            tree: false,
        }
    }
}

impl Args {
    /// Reads the arguments after the program name. The error is the message to show, which for
    /// `--help` is the usage text.
    pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut parsed = Self::default();
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            let mut value = |name: &str| {
                args.next()
                    .ok_or_else(|| format!("{name} needs a value\n\n{USAGE}"))
            };
            match arg.as_str() {
                "--help" | "-h" => return Err(USAGE.to_string()),
                "--file" => parsed.file = Some(value("--file")?.into()),
                "--screenshot" => parsed.screenshot = Some(value("--screenshot")?.into()),
                "--mode" => {
                    parsed.mode = match value("--mode")?.as_str() {
                        "normal" => Mode::Normal,
                        "insert" => Mode::Insert,
                        "visual" => Mode::Visual,
                        "visual-line" => Mode::VisualLine,
                        other => return Err(format!("unknown mode {other:?}\n\n{USAGE}")),
                    }
                }
                "--tree" => parsed.tree = true,
                "--keys" => parsed.keys = parse_keys(&value("--keys")?)?,
                "--size" => {
                    let size = value("--size")?;
                    parsed.size = size
                        .split_once('x')
                        .and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)))
                        .filter(|&(w, h)| (1..=16384).contains(&w) && (1..=16384).contains(&h))
                        .ok_or_else(|| format!("bad size {size:?}, expected WxH\n\n{USAGE}"))?;
                }
                flag if flag.starts_with("--") => {
                    return Err(format!("unknown option {flag}\n\n{USAGE}"));
                }
                _ if parsed.file.is_none() => parsed.file = Some(arg.into()),
                _ => return Err(format!("only one file can be opened\n\n{USAGE}")),
            }
        }
        Ok(parsed)
    }
}

/// Reads keys written as text with `<Name>` for the ones that have no character.
pub fn parse_keys(notation: &str) -> Result<Vec<Key>, String> {
    let mut keys = Vec::new();
    let mut rest = notation;
    while let Some(c) = rest.chars().next() {
        if c != '<' {
            keys.push(Key::Char(c));
            rest = &rest[c.len_utf8()..];
            continue;
        }
        let end = rest
            .find('>')
            .ok_or_else(|| format!("unclosed < in keys {notation:?}"))?;
        let name = &rest[1..end];
        keys.push(match name {
            "Esc" => Key::Esc,
            "CR" | "Enter" => Key::Enter,
            "BS" => Key::Backspace,
            "Del" => Key::Delete,
            "Left" => Key::Left,
            "Right" => Key::Right,
            "Up" => Key::Up,
            "Down" => Key::Down,
            "lt" => Key::Char('<'),
            _ => match name
                .strip_prefix("C-")
                .and_then(|k| k.chars().next().filter(|_| k.len() == 1))
            {
                Some(letter) => Key::Ctrl(letter.to_ascii_lowercase()),
                None => return Err(format!("unknown key <{name}>")),
            },
        });
        rest = &rest[end + 1..];
    }
    Ok(keys)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Args, String> {
        Args::parse(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn defaults_and_every_option() {
        assert_eq!(parse(&[]).unwrap(), Args::default());
        let args = parse(&[
            "--screenshot",
            "out.png",
            "--file",
            "book.dj",
            "--mode",
            "visual-line",
            "--size",
            "800x600",
        ])
        .unwrap();
        assert_eq!(args.screenshot, Some("out.png".into()));
        assert_eq!(args.file, Some("book.dj".into()));
        assert_eq!((args.mode, args.size), (Mode::VisualLine, (800, 600)));
        // A bare argument is the file.
        assert_eq!(parse(&["notes.dj"]).unwrap().file, Some("notes.dj".into()));
    }

    #[test]
    fn keys_are_written_as_text_with_names_for_special_ones() {
        assert_eq!(
            parse_keys("3jve<Esc>x<CR><C-B><lt>é").unwrap(),
            [
                Key::Char('3'),
                Key::Char('j'),
                Key::Char('v'),
                Key::Char('e'),
                Key::Esc,
                Key::Char('x'),
                Key::Enter,
                Key::Ctrl('b'),
                Key::Char('<'),
                Key::Char('é'),
            ]
        );
        assert_eq!(parse_keys("").unwrap(), []);
        for bad in ["<Esc", "<Nope>", "<C->", "<C-xy>"] {
            assert!(parse_keys(bad).is_err(), "{bad}");
        }
        assert_eq!(parse(&["--keys", "ihi<Esc>"]).unwrap().keys.len(), 4);
        assert!(parse(&["--tree"]).unwrap().tree && !parse(&[]).unwrap().tree);
    }

    #[test]
    fn bad_input_gives_a_message_and_help_gives_the_usage() {
        for args in [
            &["--mode", "vim"][..],
            &["--mode"],
            &["--size", "800"],
            &["--size", "0x600"],
            &["--size", "axb"],
            &["--size", "99999x10"],
            &["--wat"],
            &["a.dj", "b.dj"],
            &["--screenshot"],
        ] {
            assert!(parse(args).unwrap_err().contains("usage:"), "{args:?}");
        }
        assert!(parse(&["--help"]).unwrap_err().starts_with("usage:"));
        assert!(parse(&["--size", "800x"]).unwrap_err().contains("bad size"));
    }
}
