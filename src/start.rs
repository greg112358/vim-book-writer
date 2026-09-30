use crate::ui::{Align, Bounds, Chrome, Label, Rect, Theme};
use std::path::{Path, PathBuf};

/// What a click on the start screen does.
#[derive(Debug, Clone, PartialEq)]
pub enum StartAction {
    OpenFile,
    OpenFolder,
    /// A file or folder from the recent list.
    Open(PathBuf),
}

/// A place on the start screen you can click.
#[derive(Debug, Clone, PartialEq)]
pub struct StartItem {
    pub bounds: Bounds,
    pub title: String,
    /// The folder of a recent file, in smaller type under its name.
    pub detail: String,
    pub action: StartAction,
}

const COLUMN: f32 = 480.0;
const BUTTON_HEIGHT: f32 = 40.0;
const ROW_HEIGHT: f32 = 38.0;
const BUTTON_GAP: f32 = 16.0;

/// Lays out the start screen in a window `window` big: two buttons, then the recent files, as
/// many as fit, newest first.
pub fn layout(window: (f32, f32), recent: &[PathBuf]) -> Vec<StartItem> {
    let x = ((window.0 - COLUMN) / 2.0).max(16.0);
    let width = COLUMN.min(window.0 - 32.0).max(0.0);
    let top = (window.1 * 0.2).max(40.0);
    let buttons_y = top + 84.0;
    let half = ((width - BUTTON_GAP) / 2.0).max(0.0);
    let mut items = vec![
        StartItem {
            bounds: Bounds {
                x,
                y: buttons_y,
                w: half,
                h: BUTTON_HEIGHT,
            },
            title: "Open file".into(),
            detail: String::new(),
            action: StartAction::OpenFile,
        },
        StartItem {
            bounds: Bounds {
                x: x + half + BUTTON_GAP,
                y: buttons_y,
                w: half,
                h: BUTTON_HEIGHT,
            },
            title: "Open folder".into(),
            detail: String::new(),
            action: StartAction::OpenFolder,
        },
    ];
    let list_top = buttons_y + BUTTON_HEIGHT + 56.0;
    let fit = ((window.1 - list_top - 24.0) / ROW_HEIGHT).floor().max(0.0) as usize;
    for (i, path) in recent.iter().take(fit).enumerate() {
        let mut name = path.file_name().map_or_else(
            || path.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        // A folder is told from a file by the trailing slash.
        if path.is_dir() {
            name.push('/');
        }
        items.push(StartItem {
            bounds: Bounds {
                x,
                y: list_top + i as f32 * ROW_HEIGHT,
                w: width,
                h: ROW_HEIGHT,
            },
            title: name,
            detail: path.parent().map_or_else(String::new, shorten),
            action: StartAction::Open(path.clone()),
        });
    }
    items
}

/// A folder path with the home directory written as `~`.
fn shorten(dir: &Path) -> String {
    let full = dir.display().to_string();
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() && full.starts_with(&home) => {
            format!("~{}", &full[home.len()..])
        }
        _ => full,
    }
}

/// The item under (`x`, `y`).
pub fn hit(items: &[StartItem], x: f32, y: f32) -> Option<usize> {
    items.iter().position(|item| item.bounds.contains(x, y))
}

/// Draws the start screen: a title, the two buttons, and the recent list, with `hover` lit.
pub fn chrome(window: (f32, f32), items: &[StartItem], hover: Option<usize>, t: &Theme) -> Chrome {
    let mut chrome = Chrome::default();
    let x = ((window.0 - COLUMN) / 2.0).max(16.0);
    let top = (window.1 * 0.2).max(40.0);
    let label = |text: &str, x: f32, y: f32, size: f32, bold: bool, color: [u8; 4]| Label {
        text: text.into(),
        x,
        y,
        size,
        color,
        bold,
        italic: false,
        align: Align::Left,
    };
    chrome
        .labels
        .push(label("Book Writing App", x, top, 30.0, true, t.ink));
    chrome.labels.push(label(
        "A word processor for djot, with vim keys",
        x,
        top + 44.0,
        14.0,
        false,
        t.muted,
    ));
    let mut recent_heading_y = None;
    for (i, item) in items.iter().enumerate() {
        let b = item.bounds;
        let lit = hover == Some(i);
        match item.action {
            StartAction::Open(_) => {
                recent_heading_y.get_or_insert(b.y - 30.0);
                if lit {
                    chrome.rects.push(Rect::new(b, t.start_row));
                }
                chrome.labels.push(label(
                    &item.title,
                    b.x + 10.0,
                    b.y + 4.0,
                    14.0,
                    false,
                    t.ink,
                ));
                chrome.labels.push(label(
                    &item.detail,
                    b.x + 10.0,
                    b.y + 21.0,
                    11.0,
                    false,
                    t.muted,
                ));
                if i - 2 < 9 {
                    chrome.labels.push(Label {
                        align: Align::Right,
                        ..label(
                            &format!("{}", i - 1),
                            b.x + b.w - 10.0,
                            b.y + 10.0,
                            12.0,
                            false,
                            t.key_hint,
                        )
                    });
                }
            }
            _ => {
                let fill = if lit { t.start_button } else { t.bar };
                chrome.rects.push(Rect::new(b, t.border));
                chrome.rects.push(Rect::new(
                    Bounds {
                        x: b.x + 1.0,
                        y: b.y + 1.0,
                        w: b.w - 2.0,
                        h: b.h - 2.0,
                    },
                    fill,
                ));
                let hint = if item.action == StartAction::OpenFile {
                    "o"
                } else {
                    "f"
                };
                chrome.labels.push(Label {
                    align: Align::Center,
                    ..label(&item.title, b.x + b.w / 2.0, b.y + 11.0, 15.0, true, t.ink)
                });
                chrome.labels.push(Label {
                    align: Align::Right,
                    ..label(hint, b.x + b.w - 10.0, b.y + 13.0, 12.0, false, t.key_hint)
                });
            }
        }
    }
    let heading = if items
        .iter()
        .any(|i| matches!(i.action, StartAction::Open(_)))
    {
        "Open recent"
    } else {
        "No recent files yet"
    };
    let y = recent_heading_y
        .unwrap_or(items.first().map_or(top, |i| i.bounds.y) + BUTTON_HEIGHT + 20.0);
    chrome
        .labels
        .push(label(heading, x, y, 12.0, true, t.muted));
    chrome
}

/// What pressing `key` on the start screen does: `o` opens a file, `f` a folder, and `1` to `9`
/// the recent file of that number.
pub fn key_action(key: char, items: &[StartItem]) -> Option<StartAction> {
    match key {
        'o' => Some(StartAction::OpenFile),
        'f' => Some(StartAction::OpenFolder),
        '1'..='9' => items
            .iter()
            .filter(|i| matches!(i.action, StartAction::Open(_)))
            .nth(key as usize - '1' as usize)
            .map(|i| i.action.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recent(n: usize) -> Vec<PathBuf> {
        (0..n)
            .map(|i| PathBuf::from(format!("/books/novel/chapter{i}.dj")))
            .collect()
    }

    #[test]
    fn two_buttons_sit_over_the_recent_files_in_a_centered_column() {
        let items = layout((1000.0, 700.0), &recent(3));
        assert_eq!(items.len(), 5);
        assert_eq!(items[0].action, StartAction::OpenFile);
        assert_eq!(items[1].action, StartAction::OpenFolder);
        assert_eq!(
            items[2].action,
            StartAction::Open(PathBuf::from("/books/novel/chapter0.dj"))
        );
        assert_eq!(
            (items[2].title.as_str(), items[2].detail.as_str()),
            ("chapter0.dj", "/books/novel")
        );
        // Side by side, above the list, and the list in order down the page.
        assert!(items[0].bounds.x + items[0].bounds.w < items[1].bounds.x);
        assert!(items[0].bounds.y + items[0].bounds.h < items[2].bounds.y);
        assert!(items[3].bounds.y > items[2].bounds.y);
        // Centered: as much room on the left of the buttons as on the right.
        let (left, right) = (
            items[0].bounds.x,
            1000.0 - items[1].bounds.x - items[1].bounds.w,
        );
        assert!((left - right).abs() < 0.01);
    }

    #[test]
    fn only_the_recent_files_that_fit_are_listed() {
        let tall = layout((1000.0, 2000.0), &recent(20));
        assert_eq!(tall.len(), 22);
        let short = layout((1000.0, 420.0), &recent(20));
        assert!(short.len() < 22 && short.len() >= 2);
        for item in &short {
            assert!(item.bounds.y + item.bounds.h <= 420.0);
        }
        // Tiny windows and no recent files don't break anything.
        assert_eq!(layout((100.0, 50.0), &recent(5)).len(), 2);
        assert_eq!(layout((0.0, 0.0), &[]).len(), 2);
    }

    #[test]
    fn clicks_and_keys_pick_actions() {
        let items = layout((1000.0, 700.0), &recent(3));
        let center = |i: usize| (items[i].bounds.x + 5.0, items[i].bounds.y + 5.0);
        let (x, y) = center(1);
        assert_eq!(hit(&items, x, y), Some(1));
        assert_eq!(hit(&items, 5.0, 5.0), None);
        assert_eq!(key_action('o', &items), Some(StartAction::OpenFile));
        assert_eq!(key_action('f', &items), Some(StartAction::OpenFolder));
        assert_eq!(key_action('2', &items), Some(items[3].action.clone()));
        assert_eq!(key_action('9', &items), None);
        assert_eq!(key_action('x', &items), None);
    }

    #[test]
    fn the_screen_says_what_it_is_and_labels_everything() {
        let items = layout((1000.0, 700.0), &recent(2));
        let texts = |c: &Chrome| c.labels.iter().map(|l| l.text.clone()).collect::<Vec<_>>();
        let chrome = chrome((1000.0, 700.0), &items, Some(3), Theme::new(false));
        let texts = texts(&chrome);
        for want in [
            "Book Writing App",
            "Open file",
            "Open folder",
            "Open recent",
            "chapter0.dj",
            "chapter1.dj",
            "/books/novel",
        ] {
            assert!(texts.iter().any(|t| t == want), "{want:?} in {texts:?}");
        }
        // Digits go beside the recent files, and only the hovered row is lit.
        assert!(texts.contains(&"1".to_string()) && texts.contains(&"2".to_string()));
        assert_eq!(
            chrome
                .rects
                .iter()
                .filter(|r| r.color == [0xe8, 0xe1, 0xcf, 0xff])
                .count(),
            1
        );
        let empty = super::chrome(
            (1000.0, 700.0),
            &layout((1000.0, 700.0), &[]),
            None,
            Theme::new(false),
        );
        assert!(empty.labels.iter().any(|l| l.text == "No recent files yet"));
    }
}
