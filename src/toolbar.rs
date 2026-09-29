use crate::action::Action;
use crate::ui::Bounds;

/// A list that opens under a toolbar button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Menu {
    Color,
    Font,
    Size,
}

/// What a click on a toolbar button does.
#[derive(Debug, Clone, PartialEq)]
pub enum Click {
    Do(Action),
    Open(Menu),
    /// The same save `:w` does.
    Save,
    ExportMarkdown,
    ExportPdf,
    ToggleAutoSave,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Button {
    pub bounds: Bounds,
    pub label: String,
    pub click: Click,
}

/// The toolbar's buttons, left to right, in groups.
#[derive(Debug, Clone, PartialEq)]
pub struct Toolbar {
    pub buttons: Vec<Button>,
}

const GAP: f32 = 2.0;
const GROUP_GAP: f32 = 12.0;
const PADDING: f32 = 8.0;

/// Places the buttons in `bounds`, the thin strip along the top of the window.
pub fn toolbar(bounds: Bounds) -> Toolbar {
    // (label, width, click) with `None` for the space between groups.
    let mut spec: Vec<Option<(&str, f32, Click)>> = Vec::new();
    let mut add = |label, width, click| spec.push(Some((label, width, click)));
    add("B", 26.0, Click::Do(Action::Bold));
    add("I", 26.0, Click::Do(Action::Italic));
    add("U", 26.0, Click::Do(Action::Underline));
    add("S", 26.0, Click::Do(Action::Strikethrough));
    spec.push(None);
    let mut add = |label, width, click| spec.push(Some((label, width, click)));
    add("Font", 52.0, Click::Open(Menu::Font));
    add("Size", 44.0, Click::Open(Menu::Size));
    add("A", 26.0, Click::Open(Menu::Color));
    add("AA", 30.0, Click::Do(Action::Uppercase));
    spec.push(None);
    let mut add = |label, width, click| spec.push(Some((label, width, click)));
    add("H1", 28.0, Click::Do(Action::Heading(1)));
    add("H2", 28.0, Click::Do(Action::Heading(2)));
    add("H3", 28.0, Click::Do(Action::Heading(3)));
    spec.push(None);
    let mut add = |label, width, click| spec.push(Some((label, width, click)));
    add("\u{2022}", 26.0, Click::Do(Action::BulletList));
    add("1.", 26.0, Click::Do(Action::NumberedList));
    add("\"", 26.0, Click::Do(Action::Quote));
    add("Link", 40.0, Click::Do(Action::Link));

    // The buttons for the file rather than the text sit at the right end.
    let right: [(&str, f32, Click); 4] = [
        ("Save", 44.0, Click::Save),
        ("MD", 34.0, Click::ExportMarkdown),
        ("PDF", 38.0, Click::ExportPdf),
        ("Auto", 46.0, Click::ToggleAutoSave),
    ];
    let height = (bounds.h - 8.0).max(0.0);
    let y = bounds.y + (bounds.h - height) / 2.0;
    let mut x = bounds.x + PADDING;
    let mut buttons = Vec::new();
    for entry in spec {
        match entry {
            Some((label, width, click)) => {
                buttons.push(Button {
                    bounds: Bounds {
                        x,
                        y,
                        w: width,
                        h: height,
                    },
                    label: label.to_string(),
                    click,
                });
                x += width + GAP;
            }
            None => x += GROUP_GAP - GAP,
        }
    }
    let right_width: f32 = right.iter().map(|(_, w, _)| w + GAP).sum::<f32>() - GAP;
    let mut x = (bounds.x + bounds.w - PADDING - right_width).max(x + GROUP_GAP - GAP);
    for (label, width, click) in right {
        buttons.push(Button {
            bounds: Bounds {
                x,
                y,
                w: width,
                h: height,
            },
            label: label.to_string(),
            click,
        });
        x += width + GAP;
    }
    Toolbar { buttons }
}

impl Toolbar {
    /// The index of the button under (`x`, `y`).
    pub fn hit(&self, x: f32, y: f32) -> Option<usize> {
        self.buttons.iter().position(|b| b.bounds.contains(x, y))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PopupItem {
    pub bounds: Bounds,
    pub label: String,
    /// A color to show as a square instead of text.
    pub swatch: Option<[u8; 3]>,
    pub action: Action,
}

/// The list a menu button opens, with where each choice is.
#[derive(Debug, Clone, PartialEq)]
pub struct Popup {
    pub menu: Menu,
    pub bounds: Bounds,
    pub items: Vec<PopupItem>,
}

pub const COLORS: [[u8; 3]; 18] = [
    [0x00, 0x00, 0x00],
    [0x55, 0x55, 0x55],
    [0x99, 0x99, 0x99],
    [0xcc, 0x00, 0x00],
    [0xe0, 0x6c, 0x00],
    [0xc9, 0x9a, 0x00],
    [0x2e, 0x8b, 0x57],
    [0x00, 0x8b, 0x8b],
    [0x1a, 0x5f, 0xb4],
    [0x3f, 0x3f, 0xbf],
    [0x8a, 0x2b, 0xe2],
    [0xc2, 0x18, 0x5b],
    [0x8b, 0x45, 0x13],
    [0x6b, 0x8e, 0x23],
    [0x00, 0x00, 0x80],
    [0x80, 0x00, 0x00],
    [0xe7, 0x54, 0x80],
    [0xff, 0xff, 0xff],
];
pub const FONTS: [&str; 7] = [
    "Serif",
    "Sans",
    "Mono",
    "Georgia",
    "Times New Roman",
    "Arial",
    "Courier New",
];
pub const SIZES: [f32; 11] = [
    8.0, 9.0, 10.0, 11.0, 12.0, 14.0, 16.0, 18.0, 24.0, 32.0, 48.0,
];

/// Lays out the list for `menu` under `anchor`, kept inside a window `window` big.
pub fn popup(menu: Menu, anchor: Bounds, window: (f32, f32)) -> Popup {
    const PAD: f32 = 6.0;
    let mut items = Vec::new();
    // Choices are placed from (0, 0); the whole list is moved into place below.
    let size = match menu {
        Menu::Color => {
            let (cell, gap, columns) = (22.0, 3.0, 6);
            for (i, color) in COLORS.iter().enumerate() {
                let (col, row) = ((i % columns) as f32, (i / columns) as f32);
                items.push(PopupItem {
                    bounds: Bounds {
                        x: PAD + col * (cell + gap),
                        y: PAD + row * (cell + gap),
                        w: cell,
                        h: cell,
                    },
                    label: String::new(),
                    swatch: Some(*color),
                    action: Action::Color(*color),
                });
            }
            let rows = COLORS.len().div_ceil(columns) as f32;
            (
                2.0 * PAD + columns as f32 * (cell + gap) - gap,
                2.0 * PAD + rows * (cell + gap) - gap,
            )
        }
        Menu::Font | Menu::Size => {
            let (width, row) = if menu == Menu::Font {
                (170.0, 26.0)
            } else {
                (64.0, 24.0)
            };
            let labels: Vec<(String, Action)> = if menu == Menu::Font {
                FONTS
                    .iter()
                    .map(|f| (f.to_string(), Action::FontFamily(f.to_string())))
                    .collect()
            } else {
                SIZES
                    .iter()
                    .map(|&pt| (format!("{pt}"), Action::FontSize(pt)))
                    .collect()
            };
            let count = labels.len() as f32;
            for (i, (label, action)) in labels.into_iter().enumerate() {
                items.push(PopupItem {
                    bounds: Bounds {
                        x: PAD,
                        y: PAD + i as f32 * row,
                        w: width - 2.0 * PAD,
                        h: row,
                    },
                    label,
                    swatch: None,
                    action,
                });
            }
            (width, 2.0 * PAD + count * row)
        }
    };
    let x = anchor.x.min(window.0 - size.0).max(0.0);
    let y = anchor.y + anchor.h + 2.0;
    for item in &mut items {
        item.bounds.x += x;
        item.bounds.y += y;
    }
    Popup {
        menu,
        bounds: Bounds {
            x,
            y,
            w: size.0,
            h: size.1,
        },
        items,
    }
}

impl Popup {
    /// The choice under (`x`, `y`).
    pub fn hit(&self, x: f32, y: f32) -> Option<&PopupItem> {
        self.items.iter().find(|item| item.bounds.contains(x, y))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strip() -> Bounds {
        Bounds {
            x: 0.0,
            y: 0.0,
            w: 1000.0,
            h: 34.0,
        }
    }

    #[test]
    fn buttons_sit_in_the_strip_in_order_without_overlapping() {
        let toolbar = toolbar(strip());
        let labels: Vec<&str> = toolbar.buttons.iter().map(|b| b.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "B", "I", "U", "S", "Font", "Size", "A", "AA", "H1", "H2", "H3", "\u{2022}", "1.",
                "\"", "Link", "Save", "MD", "PDF", "Auto"
            ]
        );
        let strip = strip();
        for pair in toolbar.buttons.windows(2) {
            assert!(pair[0].bounds.x + pair[0].bounds.w <= pair[1].bounds.x);
        }
        for b in &toolbar.buttons {
            assert!(b.bounds.x >= strip.x && b.bounds.x + b.bounds.w <= strip.w);
            assert!(b.bounds.y >= strip.y && b.bounds.y + b.bounds.h <= strip.y + strip.h);
        }
        // The file buttons are at the right end, clear of the text buttons.
        let last = toolbar.buttons.last().unwrap().bounds;
        assert_eq!(last.x + last.w, strip.w - PADDING);
        assert_eq!(toolbar.buttons[15].click, Click::Save);
        assert_eq!(toolbar.buttons[16].click, Click::ExportMarkdown);
        assert_eq!(toolbar.buttons[17].click, Click::ExportPdf);
        assert_eq!(toolbar.buttons[18].click, Click::ToggleAutoSave);
        // Groups have a wider gap between them than buttons do.
        let gap = |i: usize| {
            toolbar.buttons[i + 1].bounds.x
                - toolbar.buttons[i].bounds.x
                - toolbar.buttons[i].bounds.w
        };
        assert!(gap(3) > gap(0) && gap(0) == GAP);
    }

    #[test]
    fn a_click_finds_its_button_and_what_it_does() {
        let toolbar = toolbar(strip());
        let center = |i: usize| {
            let b = toolbar.buttons[i].bounds;
            (b.x + b.w / 2.0, b.y + b.h / 2.0)
        };
        let (x, y) = center(0);
        let i = toolbar.hit(x, y).unwrap();
        assert_eq!(toolbar.buttons[i].click, Click::Do(Action::Bold));
        let (x, y) = center(4);
        assert_eq!(
            toolbar.buttons[toolbar.hit(x, y).unwrap()].click,
            Click::Open(Menu::Font)
        );
        // Between groups and outside the strip hit nothing.
        let b = toolbar.buttons[3].bounds;
        assert_eq!(toolbar.hit(b.x + b.w + GROUP_GAP / 2.0, 17.0), None);
        assert_eq!(toolbar.hit(700.0, 17.0), None);
        // Every formatting action has a button.
        for want in [
            Action::Bold,
            Action::Italic,
            Action::Underline,
            Action::Strikethrough,
            Action::Uppercase,
            Action::Heading(1),
            Action::Heading(2),
            Action::Heading(3),
            Action::BulletList,
            Action::NumberedList,
            Action::Quote,
            Action::Link,
        ] {
            assert!(
                toolbar
                    .buttons
                    .iter()
                    .any(|b| b.click == Click::Do(want.clone())),
                "{want:?}"
            );
        }
    }

    #[test]
    fn menus_open_under_their_button_and_stay_in_the_window() {
        let toolbar = toolbar(strip());
        for (menu, index) in [(Menu::Font, 4), (Menu::Size, 5), (Menu::Color, 6)] {
            let anchor = toolbar.buttons[index].bounds;
            let popup = popup(menu, anchor, (1000.0, 700.0));
            assert_eq!(popup.bounds.y, anchor.y + anchor.h + 2.0);
            assert!(popup.bounds.x + popup.bounds.w <= 1000.0);
            for item in &popup.items {
                let (b, p) = (item.bounds, popup.bounds);
                assert!(
                    b.x >= p.x && b.x + b.w <= p.x + p.w && b.y >= p.y && b.y + b.h <= p.y + p.h,
                    "{item:?}"
                );
            }
        }
        // Near the right edge it shifts left instead of running off.
        let popup = popup(
            Menu::Font,
            Bounds {
                x: 950.0,
                y: 4.0,
                w: 50.0,
                h: 26.0,
            },
            (1000.0, 700.0),
        );
        assert_eq!(popup.bounds.x + popup.bounds.w, 1000.0);
    }

    #[test]
    fn choosing_an_item_gives_its_action() {
        let anchor = Bounds {
            x: 100.0,
            y: 4.0,
            w: 26.0,
            h: 26.0,
        };
        let colors = popup(Menu::Color, anchor, (1000.0, 700.0));
        assert_eq!(colors.items.len(), 18);
        let red = colors
            .items
            .iter()
            .find(|i| i.swatch == Some([0xcc, 0, 0]))
            .unwrap();
        let (x, y) = (red.bounds.x + 5.0, red.bounds.y + 5.0);
        assert_eq!(
            colors.hit(x, y).unwrap().action,
            Action::Color([0xcc, 0, 0])
        );
        assert!(colors.hit(0.0, 0.0).is_none());
        let sizes = popup(Menu::Size, anchor, (1000.0, 700.0));
        assert_eq!(sizes.items[5].label, "14");
        assert_eq!(sizes.items[5].action, Action::FontSize(14.0));
        let fonts = popup(Menu::Font, anchor, (1000.0, 700.0));
        assert_eq!(fonts.items[3].action, Action::FontFamily("Georgia".into()));
        // No two choices overlap: each point picks at most one.
        for popup in [&colors, &sizes, &fonts] {
            for item in &popup.items {
                let (x, y) = (
                    item.bounds.x + item.bounds.w / 2.0,
                    item.bounds.y + item.bounds.h / 2.0,
                );
                assert_eq!(popup.hit(x, y), Some(item));
            }
        }
    }
}
