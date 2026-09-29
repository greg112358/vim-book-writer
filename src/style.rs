/// Character styling that djot can't express natively. It lives in the CSS of a
/// `{style="..."}` attribute, so HTML output needs no translation.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Style {
    pub color: Option<[u8; 3]>,
    pub size_pt: Option<f32>,
    /// The first family in the list, without quotes.
    pub family: Option<String>,
    pub underline: bool,
}

impl Style {
    /// Reads the declarations it understands (`color`, `font-size`, `font-family`,
    /// `text-decoration`). Anything malformed or unknown is skipped; a later declaration wins.
    pub fn parse(css: &str) -> Style {
        let mut style = Style::default();
        for declaration in css.split(';') {
            let Some((key, value)) = declaration.split_once(':') else {
                continue;
            };
            let (key, value) = (key.trim(), value.trim());
            if key.eq_ignore_ascii_case("color") {
                style.color = parse_hex(value).or(style.color);
            } else if key.eq_ignore_ascii_case("font-size") {
                style.size_pt = parse_size_pt(value).or(style.size_pt);
            } else if key.eq_ignore_ascii_case("font-family") {
                let first = value.split(',').next().unwrap_or("").trim();
                let first = first.trim_matches(|c| c == '"' || c == '\'').trim();
                if !first.is_empty() {
                    style.family = Some(first.to_string());
                }
            } else if key.eq_ignore_ascii_case("text-decoration") {
                style.underline = value
                    .split_whitespace()
                    .any(|word| word.eq_ignore_ascii_case("underline"));
            }
        }
        style
    }
}

/// `#rgb` or `#rrggbb`.
fn parse_hex(value: &str) -> Option<[u8; 3]> {
    let hex = value.strip_prefix('#')?;
    if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let digit = |i: usize| u8::from_str_radix(&hex[i..=i], 16).unwrap();
    match hex.len() {
        3 => Some([digit(0) * 17, digit(1) * 17, digit(2) * 17]),
        6 => Some([
            digit(0) * 16 + digit(1),
            digit(2) * 16 + digit(3),
            digit(4) * 16 + digit(5),
        ]),
        _ => None,
    }
}

/// A positive number with a `pt` or `px` unit, converted to points (96 px per 72 pt).
fn parse_size_pt(value: &str) -> Option<f32> {
    let split = value.find(|c: char| !(c.is_ascii_digit() || c == '.'))?;
    let number: f32 = value[..split].parse().ok()?;
    let unit = &value[split..];
    let pt = if unit.eq_ignore_ascii_case("pt") {
        number
    } else if unit.eq_ignore_ascii_case("px") {
        number * 0.75
    } else {
        return None;
    };
    (pt > 0.0 && pt <= 1000.0).then_some(pt)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_supported_properties() {
        let style = Style::parse(
            "color:#c00; font-size:14pt;font-family: 'Times New Roman', serif;text-decoration:underline",
        );
        assert_eq!(
            style,
            Style {
                color: Some([0xcc, 0, 0]),
                size_pt: Some(14.0),
                family: Some("Times New Roman".into()),
                underline: true,
            }
        );
    }

    #[test]
    fn converts_px_and_lets_later_declarations_win() {
        let style = Style::parse(
            "font-size:16px;text-decoration:underline;text-decoration:none;color:#FF8000;color:#0f0",
        );
        assert_eq!(style.size_pt, Some(12.0));
        assert!(!style.underline);
        assert_eq!(style.color, Some([0, 255, 0]));
    }

    #[test]
    fn malformed_values_are_ignored() {
        for css in [
            "",
            ";;;",
            "color",
            ":",
            "color:red",
            "color:#12",
            "color:#12345g",
            "color:#é12",
            "color:#+12",
            "font-size:abc",
            "font-size:12",
            "font-size:pt",
            "font-size:-3pt",
            "font-size:0pt",
            "font-size:1.2.3pt",
            "font-size:99999pt",
            "font-size:12em",
            "font-family:",
            "font-family:'', serif",
            "unknown:1;{}:;",
        ] {
            assert_eq!(Style::parse(css), Style::default(), "{css:?}");
        }
        // One bad declaration doesn't spoil its neighbours.
        assert_eq!(Style::parse("color:zzz;font-size:9pt").size_pt, Some(9.0));
    }
}
