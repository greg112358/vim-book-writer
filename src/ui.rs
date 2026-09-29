use crate::vim::Mode;
use taffy::prelude::*;

pub const TOOLBAR_HEIGHT: f32 = 34.0;
pub const STATUS_HEIGHT: f32 = 24.0;
pub const TREE_WIDTH: f32 = 260.0;

/// A rectangle of the window, in pixels from its top left.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Bounds {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Bounds {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}

/// Where the parts of the window go: a thin toolbar across the top, a status bar across the
/// bottom, and between them the editor with the file tree on its right.
#[derive(Debug, Clone, PartialEq)]
pub struct Regions {
    pub toolbar: Bounds,
    pub editor: Bounds,
    pub tree: Option<Bounds>,
    pub status: Bounds,
}

/// Lays out a window `width` by `height`. The tree takes a fixed width and the editor the rest.
pub fn regions(width: f32, height: f32, tree: bool) -> Regions {
    let fixed = |width: f32, height: f32| Style {
        size: Size {
            width: if width < 0.0 {
                Dimension::AUTO
            } else {
                Dimension::from_length(width)
            },
            height: if height < 0.0 {
                Dimension::AUTO
            } else {
                Dimension::from_length(height)
            },
        },
        flex_shrink: 0.0,
        ..Default::default()
    };
    let mut taffy: TaffyTree<()> = TaffyTree::new();
    let toolbar = taffy.new_leaf(fixed(-1.0, TOOLBAR_HEIGHT)).unwrap();
    let editor = taffy
        .new_leaf(Style {
            flex_grow: 1.0,
            min_size: Size {
                width: LengthPercentageAuto::from_length(0.0),
                height: LengthPercentageAuto::AUTO,
            },
            ..Default::default()
        })
        .unwrap();
    let tree_node = tree.then(|| taffy.new_leaf(fixed(TREE_WIDTH, -1.0)).unwrap());
    let mut row = vec![editor];
    row.extend(tree_node);
    let body = taffy
        .new_with_children(
            Style {
                flex_direction: FlexDirection::Row,
                flex_grow: 1.0,
                min_size: Size {
                    width: LengthPercentageAuto::AUTO,
                    height: LengthPercentageAuto::from_length(0.0),
                },
                ..Default::default()
            },
            &row,
        )
        .unwrap();
    let status = taffy.new_leaf(fixed(-1.0, STATUS_HEIGHT)).unwrap();
    let root = taffy
        .new_with_children(
            Style {
                flex_direction: FlexDirection::Column,
                size: Size {
                    width: Dimension::from_length(width),
                    height: Dimension::from_length(height),
                },
                ..Default::default()
            },
            &[toolbar, body, status],
        )
        .unwrap();
    taffy
        .compute_layout(
            root,
            Size {
                width: AvailableSpace::Definite(width),
                height: AvailableSpace::Definite(height),
            },
        )
        .unwrap();
    let bounds = |node, origin: (f32, f32)| {
        let layout = taffy.layout(node).unwrap();
        Bounds {
            x: origin.0 + layout.location.x,
            y: origin.1 + layout.location.y,
            w: layout.size.width,
            h: layout.size.height,
        }
    };
    let body_bounds = bounds(body, (0.0, 0.0));
    let origin = (body_bounds.x, body_bounds.y);
    Regions {
        toolbar: bounds(toolbar, (0.0, 0.0)),
        editor: bounds(editor, origin),
        tree: tree_node.map(|node| bounds(node, origin)),
        status: bounds(status, (0.0, 0.0)),
    }
}

/// A filled rectangle in pixels from the top left of the frame, in sRGB with alpha.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub color: [u8; 4],
}

impl Rect {
    pub fn new(bounds: Bounds, color: [u8; 4]) -> Self {
        Self {
            x: bounds.x,
            y: bounds.y,
            w: bounds.w,
            h: bounds.h,
            color,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Align {
    Left,
    Right,
    Center,
}

/// A line of interface text. `x` is its left edge, right edge, or middle, by `align`, and `y` is
/// the top of the line.
#[derive(Debug, Clone, PartialEq)]
pub struct Label {
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub size: f32,
    pub color: [u8; 4],
    pub bold: bool,
    pub align: Align,
}

/// The window around the editor, as shapes and text for the renderer to draw.
#[derive(Debug, Default, PartialEq)]
pub struct Chrome {
    pub rects: Vec<Rect>,
    pub labels: Vec<Label>,
}

pub const BAR: [u8; 4] = [0xee, 0xe8, 0xda, 0xff];
pub const PANEL: [u8; 4] = [0xf2, 0xed, 0xe1, 0xff];
pub const BORDER: [u8; 4] = [0xd8, 0xd0, 0xbd, 0xff];
pub const INK: [u8; 4] = [0x33, 0x33, 0x33, 0xff];
pub const MUTED: [u8; 4] = [0x7a, 0x74, 0x66, 0xff];
const LABEL_SIZE: f32 = 12.0;

/// What the status bar says.
#[derive(Debug, Clone, PartialEq)]
pub struct Status {
    pub mode: Mode,
    pub file: String,
    pub modified: bool,
    pub words: usize,
    /// A note from the editor, like a failed search, shown in place of the file.
    pub message: String,
}

fn mode_name(mode: Mode) -> &'static str {
    match mode {
        Mode::Normal => "NORMAL",
        Mode::Insert => "INSERT",
        Mode::Visual => "VISUAL",
        Mode::VisualLine => "V-LINE",
    }
}

/// The toolbar and status bar backgrounds, the tree panel, and the status text.
pub fn chrome(regions: &Regions, status: &Status) -> Chrome {
    let mut chrome = Chrome::default();
    let line = |bounds: Bounds| Rect {
        h: 1.0,
        ..Rect::new(bounds, BORDER)
    };

    let toolbar = regions.toolbar;
    chrome.rects.push(Rect::new(toolbar, BAR));
    chrome.rects.push(Rect {
        y: toolbar.y + toolbar.h - 1.0,
        ..line(toolbar)
    });

    if let Some(tree) = regions.tree {
        chrome.rects.push(Rect::new(tree, PANEL));
        chrome.rects.push(Rect {
            w: 1.0,
            ..Rect::new(tree, BORDER)
        });
    }

    let bar = regions.status;
    chrome.rects.push(Rect::new(bar, BAR));
    chrome.rects.push(line(bar));
    let text_y = bar.y + (bar.h - LABEL_SIZE * 1.3) / 2.0;
    let label = |text: String, x: f32, align: Align, bold: bool, color: [u8; 4]| Label {
        text,
        x,
        y: text_y,
        size: LABEL_SIZE,
        color,
        bold,
        align,
    };
    chrome.labels.push(label(
        mode_name(status.mode).into(),
        bar.x + 12.0,
        Align::Left,
        true,
        INK,
    ));
    let file = if status.message.is_empty() {
        format!(
            "{}{}",
            status.file,
            if status.modified { " [+]" } else { "" }
        )
    } else {
        status.message.clone()
    };
    chrome
        .labels
        .push(label(file, bar.x + 90.0, Align::Left, false, INK));
    let words = format!(
        "{} {}",
        status.words,
        if status.words == 1 { "word" } else { "words" }
    );
    chrome.labels.push(label(
        words,
        bar.x + bar.w - 12.0,
        Align::Right,
        false,
        MUTED,
    ));
    chrome
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_editor_fills_what_the_bars_and_tree_leave() {
        let r = regions(1000.0, 700.0, false);
        assert_eq!(
            r.toolbar,
            Bounds {
                x: 0.0,
                y: 0.0,
                w: 1000.0,
                h: TOOLBAR_HEIGHT
            }
        );
        assert_eq!(
            r.status,
            Bounds {
                x: 0.0,
                y: 700.0 - STATUS_HEIGHT,
                w: 1000.0,
                h: STATUS_HEIGHT
            }
        );
        assert_eq!(
            r.editor,
            Bounds {
                x: 0.0,
                y: TOOLBAR_HEIGHT,
                w: 1000.0,
                h: 700.0 - TOOLBAR_HEIGHT - STATUS_HEIGHT
            }
        );
        assert_eq!(r.tree, None);

        // The tree is on the right and takes its width from the editor.
        let r = regions(1000.0, 700.0, true);
        let tree = r.tree.unwrap();
        assert_eq!(
            tree,
            Bounds {
                x: 1000.0 - TREE_WIDTH,
                y: TOOLBAR_HEIGHT,
                w: TREE_WIDTH,
                h: r.editor.h
            }
        );
        assert_eq!(r.editor.w, 1000.0 - TREE_WIDTH);
        assert_eq!(r.editor.x + r.editor.w, tree.x);
    }

    #[test]
    fn a_tiny_window_doesnt_make_negative_sizes() {
        for (w, h) in [(100.0, 50.0), (0.0, 0.0), (300.0, 20.0)] {
            let r = regions(w, h, true);
            for b in [Some(r.toolbar), Some(r.editor), r.tree, Some(r.status)]
                .into_iter()
                .flatten()
            {
                assert!(b.w >= 0.0 && b.h >= 0.0, "{w}x{h}: {b:?}");
            }
        }
    }

    #[test]
    fn bounds_contain_their_top_left_but_not_their_far_edge() {
        let b = Bounds {
            x: 10.0,
            y: 20.0,
            w: 30.0,
            h: 40.0,
        };
        assert!(b.contains(10.0, 20.0) && b.contains(39.9, 59.9));
        assert!(!b.contains(40.0, 30.0) && !b.contains(20.0, 60.0) && !b.contains(9.9, 30.0));
    }

    #[test]
    fn the_status_bar_shows_mode_file_changes_and_words() {
        let regions = regions(800.0, 600.0, false);
        let status = |mode, modified, words, message: &str| Status {
            mode,
            file: "book.dj".into(),
            modified,
            words,
            message: message.into(),
        };
        let texts = |s: Status| -> Vec<String> {
            chrome(&regions, &s)
                .labels
                .into_iter()
                .map(|l| l.text)
                .collect()
        };
        assert_eq!(
            texts(status(Mode::Normal, false, 1234, "")),
            ["NORMAL", "book.dj", "1234 words"]
        );
        assert_eq!(
            texts(status(Mode::Insert, true, 1, "")),
            ["INSERT", "book.dj [+]", "1 word"]
        );
        assert_eq!(texts(status(Mode::VisualLine, false, 0, ""))[0], "V-LINE");
        assert_eq!(
            texts(status(Mode::Visual, true, 5, "E486: Pattern not found: x"))[1],
            "E486: Pattern not found: x"
        );
        // The labels sit inside the status bar.
        let bar = regions.status;
        for label in chrome(&regions, &status(Mode::Normal, false, 3, "")).labels {
            assert!(
                label.y >= bar.y && label.y + label.size <= bar.y + bar.h,
                "{label:?}"
            );
        }
    }
}
