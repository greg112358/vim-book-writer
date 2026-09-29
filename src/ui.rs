use crate::toolbar::{Click, Menu, Popup, Toolbar};
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
    pub italic: bool,
    pub align: Align,
}

/// The window around the editor, as shapes and text for the renderer to draw.
#[derive(Debug, Default, PartialEq)]
pub struct Chrome {
    pub rects: Vec<Rect>,
    pub labels: Vec<Label>,
    /// Drawn on top of the editor's text and its caret, like an open menu.
    pub over_rects: Vec<Rect>,
    pub over_labels: Vec<Label>,
}

pub const TREE_HEADER: f32 = 34.0;
pub const TREE_ROW: f32 = 22.0;

/// How many tree rows fit in the panel.
pub fn tree_capacity(panel: Bounds) -> usize {
    ((panel.h - TREE_HEADER) / TREE_ROW).floor().max(0.0) as usize
}

/// The row of the tree panel under (`x`, `y`), counting from `first`, the first row shown.
pub fn tree_row_at(panel: Bounds, first: usize, x: f32, y: f32) -> Option<usize> {
    let below = y - panel.y - TREE_HEADER;
    (panel.contains(x, y) && below >= 0.0).then(|| first + (below / TREE_ROW) as usize)
}

/// One line of the file tree.
#[derive(Debug, Clone, PartialEq)]
pub struct TreeRow {
    pub name: String,
    pub depth: usize,
    pub is_dir: bool,
    pub expanded: bool,
    pub selected: bool,
}

/// The file tree as it is now: the folder's name, the rows that fit from the first one showing,
/// and whether it has the keyboard.
#[derive(Debug, Clone, PartialEq)]
pub struct TreeView {
    pub header: String,
    pub rows: Vec<TreeRow>,
    pub focused: bool,
}

/// The toolbar as it is now, for `chrome` to draw.
pub struct Interface<'a> {
    pub toolbar: &'a Toolbar,
    /// The button the mouse is over.
    pub hover: Option<usize>,
    pub popup: Option<&'a Popup>,
    /// The color last chosen, shown under the color button.
    pub color: [u8; 3],
    pub tree: Option<&'a TreeView>,
}

pub const BAR: [u8; 4] = [0xee, 0xe8, 0xda, 0xff];
pub const PANEL: [u8; 4] = [0xf2, 0xed, 0xe1, 0xff];
pub const BORDER: [u8; 4] = [0xd8, 0xd0, 0xbd, 0xff];
pub const INK: [u8; 4] = [0x33, 0x33, 0x33, 0xff];
pub const MUTED: [u8; 4] = [0x7a, 0x74, 0x66, 0xff];
const LABEL_SIZE: f32 = 12.0;
const BUTTON_SIZE: f32 = 13.0;
const HOVER: [u8; 4] = [0xe0, 0xd8, 0xc4, 0xff];
const MENU: [u8; 4] = [0xfc, 0xf9, 0xf2, 0xff];

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

/// The toolbar and its buttons, the status bar and its text, the tree panel, and an open menu.
pub fn chrome(regions: &Regions, status: &Status, ui: &Interface<'_>) -> Chrome {
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
    for (i, button) in ui.toolbar.buttons.iter().enumerate() {
        let b = button.bounds;
        let open =
            matches!((&button.click, ui.popup), (Click::Open(menu), Some(p)) if p.menu == *menu);
        if ui.hover == Some(i) || open {
            chrome.rects.push(Rect::new(b, HOVER));
        }
        let is_color = button.click == Click::Open(Menu::Color);
        chrome.labels.push(Label {
            text: button.label.clone(),
            x: b.x + b.w / 2.0,
            // The color button's letter sits higher, to leave room for the swatch under it.
            y: b.y + (b.h - BUTTON_SIZE * 1.3) / 2.0 - if is_color { 2.0 } else { 0.0 },
            size: BUTTON_SIZE,
            color: INK,
            bold: matches!(button.label.as_str(), "B" | "H1" | "H2" | "H3"),
            italic: button.label == "I",
            align: Align::Center,
        });
        if is_color {
            let [r, g, blue] = ui.color;
            chrome.rects.push(Rect {
                x: b.x + 6.0,
                y: b.y + b.h - 8.0,
                w: b.w - 12.0,
                h: 3.0,
                color: [r, g, blue, 0xff],
            });
        }
    }
    if let Some(popup) = ui.popup {
        let p = popup.bounds;
        chrome.over_rects.push(Rect {
            x: p.x + 2.0,
            y: p.y + 2.0,
            ..Rect::new(p, [0, 0, 0, 0x28])
        });
        chrome.over_rects.push(Rect::new(p, BORDER));
        chrome.over_rects.push(Rect::new(
            Bounds {
                x: p.x + 1.0,
                y: p.y + 1.0,
                w: p.w - 2.0,
                h: p.h - 2.0,
            },
            MENU,
        ));
        for item in &popup.items {
            let b = item.bounds;
            if let Some([r, g, blue]) = item.swatch {
                chrome
                    .over_rects
                    .push(Rect::new(b, [0x99, 0x92, 0x84, 0xff]));
                chrome.over_rects.push(Rect::new(
                    Bounds {
                        x: b.x + 1.0,
                        y: b.y + 1.0,
                        w: b.w - 2.0,
                        h: b.h - 2.0,
                    },
                    [r, g, blue, 0xff],
                ));
            } else {
                chrome.over_labels.push(Label {
                    text: item.label.clone(),
                    x: b.x + 6.0,
                    y: b.y + (b.h - BUTTON_SIZE * 1.3) / 2.0,
                    size: BUTTON_SIZE,
                    color: INK,
                    bold: false,
                    italic: false,
                    align: Align::Left,
                });
            }
        }
    }

    if let Some(tree) = regions.tree {
        chrome.rects.push(Rect::new(tree, PANEL));
        chrome.rects.push(Rect {
            w: 1.0,
            ..Rect::new(tree, BORDER)
        });
        if let Some(view) = ui.tree {
            let text = |text: String, x: f32, y: f32, bold: bool, color: [u8; 4]| Label {
                text,
                x,
                y,
                size: 13.0,
                color,
                bold,
                italic: false,
                align: Align::Left,
            };
            chrome.labels.push(text(
                view.header.clone(),
                tree.x + 12.0,
                tree.y + 9.0,
                true,
                INK,
            ));
            chrome.rects.push(Rect {
                y: tree.y + TREE_HEADER - 2.0,
                x: tree.x + 1.0,
                w: tree.w - 1.0,
                ..line(tree)
            });
            for (i, row) in view.rows.iter().enumerate() {
                let y = tree.y + TREE_HEADER + i as f32 * TREE_ROW;
                if row.selected {
                    let fill = if view.focused {
                        [0xd9, 0xcf, 0xb4, 0xff]
                    } else {
                        [0xe6, 0xdf, 0xcd, 0xff]
                    };
                    let bounds = Bounds {
                        x: tree.x + 1.0,
                        y,
                        w: tree.w - 1.0,
                        h: TREE_ROW,
                    };
                    chrome.rects.push(Rect::new(bounds, fill));
                }
                let marker = match (row.is_dir, row.expanded) {
                    (true, true) => "\u{25be} ",
                    (true, false) => "\u{25b8} ",
                    _ => "   ",
                };
                let color = if row.is_dir {
                    INK
                } else {
                    [0x44, 0x40, 0x38, 0xff]
                };
                chrome.labels.push(text(
                    format!("{marker}{}", row.name),
                    tree.x + 10.0 + row.depth as f32 * 14.0,
                    y + 3.0,
                    row.is_dir,
                    color,
                ));
            }
        }
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
        italic: false,
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
    fn tree_rows_are_found_by_position() {
        let panel = Bounds {
            x: 700.0,
            y: 34.0,
            w: 260.0,
            h: TREE_HEADER + 10.0 * TREE_ROW,
        };
        assert_eq!(tree_capacity(panel), 10);
        assert_eq!(tree_capacity(Bounds { h: 10.0, ..panel }), 0);
        let y = |row: f32| panel.y + TREE_HEADER + row * TREE_ROW;
        assert_eq!(tree_row_at(panel, 0, 710.0, y(0.0) + 1.0), Some(0));
        assert_eq!(tree_row_at(panel, 0, 710.0, y(3.0) + 5.0), Some(3));
        // Counted from the first row that is showing, and not in the header or outside the panel.
        assert_eq!(tree_row_at(panel, 7, 710.0, y(2.0) + 5.0), Some(9));
        assert_eq!(tree_row_at(panel, 0, 710.0, panel.y + 5.0), None);
        assert_eq!(tree_row_at(panel, 0, 100.0, y(1.0)), None);
    }

    #[test]
    fn the_tree_panel_shows_its_header_rows_and_selection() {
        let regions = regions(1000.0, 700.0, true);
        let toolbar = crate::toolbar::Toolbar {
            buttons: Vec::new(),
        };
        let row = |name: &str, depth, is_dir, expanded, selected| TreeRow {
            name: name.into(),
            depth,
            is_dir,
            expanded,
            selected,
        };
        let view = TreeView {
            header: "novel".into(),
            rows: vec![
                row("chapters", 0, true, true, false),
                row("one.dj", 1, false, false, true),
                row("notes", 0, true, false, false),
            ],
            focused: true,
        };
        let status = Status {
            mode: Mode::Normal,
            file: "f".into(),
            modified: false,
            words: 0,
            message: String::new(),
        };
        let interface = Interface {
            toolbar: &toolbar,
            hover: None,
            popup: None,
            color: [0; 3],
            tree: Some(&view),
        };
        let chrome = chrome(&regions, &status, &interface);
        let labels: Vec<&str> = chrome.labels.iter().map(|l| l.text.as_str()).collect();
        for want in ["novel", "\u{25be} chapters", "   one.dj", "\u{25b8} notes"] {
            assert!(labels.contains(&want), "{want:?} in {labels:?}");
        }
        // Children are indented past their folder, and the selected row has a highlight behind it.
        let x = |text: &str| chrome.labels.iter().find(|l| l.text == text).unwrap().x;
        assert!(x("   one.dj") > x("\u{25be} chapters"));
        let one = chrome
            .labels
            .iter()
            .find(|l| l.text == "   one.dj")
            .unwrap();
        assert!(
            chrome
                .rects
                .iter()
                .any(|r| r.y <= one.y && one.y < r.y + r.h && r.color == [0xd9, 0xcf, 0xb4, 0xff])
        );
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
        let toolbar = crate::toolbar::Toolbar {
            buttons: Vec::new(),
        };
        let interface = Interface {
            toolbar: &toolbar,
            hover: None,
            popup: None,
            color: [0, 0, 0],
            tree: None,
        };
        let texts = |s: Status| -> Vec<String> {
            chrome(&regions, &s, &interface)
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
        let toolbar = crate::toolbar::Toolbar {
            buttons: Vec::new(),
        };
        let interface = Interface {
            toolbar: &toolbar,
            hover: None,
            popup: None,
            color: [0, 0, 0],
            tree: None,
        };
        for label in chrome(&regions, &status(Mode::Normal, false, 3, ""), &interface).labels {
            assert!(
                label.y >= bar.y && label.y + label.size <= bar.y + bar.h,
                "{label:?}"
            );
        }
    }
}
