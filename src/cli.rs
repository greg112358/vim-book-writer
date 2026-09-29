use crate::vim::Mode;
use std::path::PathBuf;

const USAGE: &str =
    "usage: book-writing-app [FILE] [--file FILE] [--screenshot PNG [--mode MODE] [--size WxH]]

  FILE, --file FILE   the djot document to open
  --screenshot PNG    draw one frame offscreen to PNG and exit
  --mode MODE         the mode to show: normal, insert, visual, or visual-line
  --size WxH          the size of the screenshot in pixels (default 1000x700)";

/// What the command line asks for.
#[derive(Debug, Clone, PartialEq)]
pub struct Args {
    pub file: Option<PathBuf>,
    /// If set, render one frame to this PNG instead of opening a window.
    pub screenshot: Option<PathBuf>,
    pub mode: Mode,
    pub size: (u32, u32),
}

impl Default for Args {
    fn default() -> Self {
        Self {
            file: None,
            screenshot: None,
            mode: Mode::Normal,
            size: (1000, 700),
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
