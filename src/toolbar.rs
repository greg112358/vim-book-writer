use crate::action::Action;
use crate::ui::Bounds;

/// A list that opens under a toolbar button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Menu {
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
    /// Switch between the light and dark looks.
    ToggleDark,
    /// Show or hide the outline of headings.
    ToggleOutline,
    /// Show the list of keys.
    Help,
    /// A file tree button: the key it stands for.
    TreeKey(char),
    /// A file tree git button: the key after `g` that it stands for.
    TreeGit(char),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Button {
    pub bounds: Bounds,
    pub label: String,
    /// What the button does and its shortcut, shown when the mouse rests on it.
    pub tip: String,
    pub click: Click,
}

/// The toolbar's buttons, left to right, in groups.
#[derive(Debug, Clone, PartialEq)]
pub struct Toolbar {
    pub buttons: Vec<Button>,
}

const GAP: f32 = 2.0;
const GROUP_GAP: f32 = 14.0;
const PADDING: f32 = 10.0;
/// How much wider each button is drawn than the widths written below, which suit small type.
const SCALE: f32 = 1.1;

/// Places the buttons in `bounds`, the thin strip along the top of the window.
pub fn toolbar(bounds: Bounds) -> Toolbar {
    let cmd = if cfg!(target_os = "macos") {
        "Cmd"
    } else {
        "Ctrl"
    };
    // (label, width, click, tip) with `None` for the space between groups.
    let mut spec: Vec<Option<(&str, f32, Click, String)>> = Vec::new();
    let mut add =
        |label, width, click, tip: &str| spec.push(Some((label, width, click, tip.to_string())));
    add("B", 26.0, Click::Do(Action::Bold), "Bold (Ctrl+B)");
    add("I", 26.0, Click::Do(Action::Italic), "Italic (Ctrl+I)");
    add("S", 26.0, Click::Do(Action::Strikethrough), "Strikethrough");
    add(
        "x\u{b2}",
        28.0,
        Click::Do(Action::Superscript),
        "Superscript",
    );
    add("x\u{2082}", 28.0, Click::Do(Action::Subscript), "Subscript");
    add("AA", 30.0, Click::Do(Action::Uppercase), "Make uppercase");
    spec.push(None);
    let mut add =
        |label, width, click, tip: &str| spec.push(Some((label, width, click, tip.to_string())));
    add(
        "H1",
        28.0,
        Click::Do(Action::Heading(1)),
        "Heading 1 (again for plain text)",
    );
    add(
        "H2",
        28.0,
        Click::Do(Action::Heading(2)),
        "Heading 2 (again for plain text)",
    );
    add(
        "H3",
        28.0,
        Click::Do(Action::Heading(3)),
        "Heading 3 (again for plain text)",
    );
    spec.push(None);
    let mut add =
        |label, width, click, tip: &str| spec.push(Some((label, width, click, tip.to_string())));
    add(
        "\u{2022}",
        26.0,
        Click::Do(Action::BulletList),
        "Bulleted list",
    );
    add("1.", 26.0, Click::Do(Action::NumberedList), "Numbered list");
    add("\"", 26.0, Click::Do(Action::Quote), "Block quote");
    add("Link", 40.0, Click::Do(Action::Link), "Link");

    // The buttons for the file rather than the text sit at the right end.
    let right: [(&str, f32, Click, String); 9] = [
        (
            "Font",
            52.0,
            Click::Open(Menu::Font),
            "Reading font: how the text looks to you, not in the file".into(),
        ),
        (
            "Size",
            44.0,
            Click::Open(Menu::Size),
            "Reading size: how big the text looks to you, not in the file".into(),
        ),
        (
            "Outline",
            52.0,
            Click::ToggleOutline,
            "Show or hide the headings (Ctrl+H goes there)".into(),
        ),
        ("Save", 40.0, Click::Save, "Save (Ctrl+S or :w)".into()),
        (
            "MD",
            30.0,
            Click::ExportMarkdown,
            "Export as Markdown".into(),
        ),
        ("PDF", 34.0, Click::ExportPdf, "Export as PDF".into()),
        (
            "Auto",
            40.0,
            Click::ToggleAutoSave,
            "Auto-save after you stop typing".into(),
        ),
        (
            "\u{263e}",
            26.0,
            Click::ToggleDark,
            "Switch between light and dark".into(),
        ),
        ("?", 26.0, Click::Help, "Every key (F1)".into()),
    ];
    let height = (bounds.h - 8.0).max(0.0);
    let y = bounds.y + (bounds.h - height) / 2.0;
    let mut x = bounds.x + PADDING;
    let mut buttons = Vec::new();
    for entry in spec {
        match entry {
            Some((label, width, click, tip)) => {
                buttons.push(Button {
                    bounds: Bounds {
                        x,
                        y,
                        w: width * SCALE,
                        h: height,
                    },
                    label: label.to_string(),
                    tip: tip.replace("Ctrl", cmd),
                    click,
                });
                x += width * SCALE + GAP;
            }
            None => x += GROUP_GAP - GAP,
        }
    }
    let right_width: f32 = right
        .iter()
        .map(|(_, w, _, _)| w * SCALE + GAP)
        .sum::<f32>()
        - GAP;
    let mut x = (bounds.x + bounds.w - PADDING - right_width).max(x + GROUP_GAP - GAP);
    for (label, width, click, tip) in right {
        buttons.push(Button {
            bounds: Bounds {
                x,
                y,
                w: width * SCALE,
                h: height,
            },
            label: label.to_string(),
            tip: tip.replace("Ctrl", cmd),
            click,
        });
        x += width * SCALE + GAP;
    }
    Toolbar { buttons }
}

/// The buttons under the file tree's header for the panel `panel`: two rows for files, and for a
/// repository two more for git under the branch line.
pub fn tree_buttons(panel: Bounds, git: bool) -> Toolbar {
    const ROWS: [&[(&str, f32, char, &str)]; 2] = [
        &[
            ("+File", 42.0, 'a', "New file (a)"),
            ("+Folder", 52.0, 'A', "New folder (A)"),
            ("Rename", 50.0, 'r', "Rename (r)"),
            ("Delete", 46.0, 'd', "Delete, after asking (d)"),
            ("Copy", 38.0, 'c', "Copy to a path (c)"),
        ],
        &[
            ("Move", 38.0, 'm', "Move to a path (m)"),
            ("Yank", 38.0, 'y', "Yank, to paste a copy (y)"),
            ("Paste", 42.0, 'p', "Paste a copy of what was yanked (p)"),
        ],
    ];
    const GIT_ROWS: [&[(&str, f32, char, &str)]; 2] = [
        &[
            ("Stage", 42.0, 'a', "Stage the selected file or folder (ga)"),
            ("Unstage", 52.0, 'u', "Unstage it (gu)"),
            (
                "Discard",
                50.0,
                'r',
                "Throw away its changes, after asking (gr)",
            ),
            ("Commit", 48.0, 'c', "Commit what is staged (gc)"),
        ],
        &[
            ("Stage all", 62.0, 'A', "Stage every change (gA)"),
            ("Push", 38.0, 'p', "Push the branch (gp)"),
            ("Pull", 38.0, 'l', "Pull the branch (gl)"),
        ],
    ];
    let mut buttons = Vec::new();
    let mut place = |rows: &[&[(&str, f32, char, &str)]], top: f32, git: bool| {
        for (row, entries) in rows.iter().enumerate() {
            let mut x = panel.x + 10.0;
            for &(label, w, key, tip) in *entries {
                // Drawn a little wider than the widths above, which suit smaller type.
                let w = w * 1.15;
                buttons.push(Button {
                    bounds: Bounds {
                        x,
                        y: panel.y + top + row as f32 * crate::ui::TREE_BUTTON_ROW,
                        w,
                        h: 26.0,
                    },
                    label: label.into(),
                    tip: tip.into(),
                    click: if git {
                        Click::TreeGit(key)
                    } else {
                        Click::TreeKey(key)
                    },
                });
                x += w + 4.0;
            }
        }
    };
    place(&ROWS, crate::ui::TREE_HEADER, false);
    if git {
        let top = crate::ui::TREE_HEADER + 2.0 * crate::ui::TREE_BUTTON_ROW + crate::ui::GIT_LINE;
        place(&GIT_ROWS, top, true);
    }
    Toolbar { buttons }
}

impl Toolbar {
    /// The index of the button under (`x`, `y`).
    pub fn hit(&self, x: f32, y: f32) -> Option<usize> {
        self.buttons.iter().position(|b| b.bounds.contains(x, y))
    }
}

/// What choosing an item in a menu sets: the font the text is read in, or its size in points.
#[derive(Debug, Clone, PartialEq)]
pub enum Choice {
    Font(String),
    Size(f32),
}

#[derive(Debug, Clone, PartialEq)]
pub struct PopupItem {
    pub bounds: Bounds,
    pub label: String,
    pub choice: Choice,
    /// The choice that is in force now, shown marked.
    pub selected: bool,
}

/// The list a menu button opens, with where each choice is.
#[derive(Debug, Clone, PartialEq)]
pub struct Popup {
    pub menu: Menu,
    pub bounds: Bounds,
    pub items: Vec<PopupItem>,
}

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
    const PAD: f32 = 8.0;
    let mut items = Vec::new();
    // Choices are placed from (0, 0); the whole list is moved into place below.
    let size = {
        let (width, row) = if menu == Menu::Font {
            (210.0, 32.0)
        } else {
            (84.0, 30.0)
        };
        let labels: Vec<(String, Choice)> = if menu == Menu::Font {
            FONTS
                .iter()
                .map(|f| (f.to_string(), Choice::Font(f.to_string())))
                .collect()
        } else {
            SIZES
                .iter()
                .map(|&pt| (format!("{pt}"), Choice::Size(pt)))
                .collect()
        };
        let count = labels.len() as f32;
        for (i, (label, choice)) in labels.into_iter().enumerate() {
            items.push(PopupItem {
                bounds: Bounds {
                    x: PAD,
                    y: PAD + i as f32 * row,
                    w: width - 2.0 * PAD,
                    h: row,
                },
                label,
                choice,
                selected: false,
            });
        }
        (width, 2.0 * PAD + count * row)
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
    /// Marks the choice named `label`, ignoring case, as the one in force.
    pub fn mark(&mut self, label: &str) {
        for item in &mut self.items {
            item.selected = item.label.eq_ignore_ascii_case(label);
        }
    }

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
                "B",
                "I",
                "S",
                "x\u{b2}",
                "x\u{2082}",
                "AA",
                "H1",
                "H2",
                "H3",
                "\u{2022}",
                "1.",
                "\"",
                "Link",
                "Font",
                "Size",
                "Outline",
                "Save",
                "MD",
                "PDF",
                "Auto",
                "\u{263e}",
                "?"
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
        assert!((last.x + last.w - (strip.w - PADDING)).abs() < 0.01);
        assert_eq!(toolbar.buttons[15].click, Click::ToggleOutline);
        assert_eq!(toolbar.buttons[16].click, Click::Save);
        assert_eq!(toolbar.buttons[17].click, Click::ExportMarkdown);
        assert_eq!(toolbar.buttons[18].click, Click::ExportPdf);
        assert_eq!(toolbar.buttons[19].click, Click::ToggleAutoSave);
        assert_eq!(toolbar.buttons[20].click, Click::ToggleDark);
        assert_eq!(toolbar.buttons[21].click, Click::Help);
        assert_eq!(toolbar.buttons[0].tip.split(' ').next(), Some("Bold"));
        assert!(toolbar.buttons.iter().all(|b| !b.tip.is_empty()));
        // Groups have a wider gap between them than buttons do.
        let gap = |i: usize| {
            toolbar.buttons[i + 1].bounds.x
                - toolbar.buttons[i].bounds.x
                - toolbar.buttons[i].bounds.w
        };
        assert!(gap(5) > gap(0) && (gap(0) - GAP).abs() < 0.01);
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
        let (x, y) = center(13);
        assert_eq!(
            toolbar.buttons[toolbar.hit(x, y).unwrap()].click,
            Click::Open(Menu::Font)
        );
        // Between groups and outside the strip hit nothing.
        let b = toolbar.buttons[5].bounds;
        assert_eq!(toolbar.hit(b.x + b.w + GROUP_GAP / 2.0, 17.0), None);
        let link = toolbar.buttons[12].bounds;
        assert_eq!(toolbar.hit(link.x + link.w + 2.0, 17.0), None);
        assert_eq!(toolbar.hit(strip().w + 5.0, 17.0), None);
        // Every formatting action has a button.
        for want in [
            Action::Bold,
            Action::Italic,
            Action::Strikethrough,
            Action::Superscript,
            Action::Subscript,
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
        for (menu, index) in [(Menu::Font, 13), (Menu::Size, 14)] {
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
    fn choosing_an_item_gives_its_setting() {
        let anchor = Bounds {
            x: 100.0,
            y: 4.0,
            w: 26.0,
            h: 26.0,
        };
        let sizes = popup(Menu::Size, anchor, (1000.0, 700.0));
        assert_eq!(sizes.items[5].label, "14");
        assert_eq!(sizes.items[5].choice, Choice::Size(14.0));
        let mut fonts = popup(Menu::Font, anchor, (1000.0, 700.0));
        assert_eq!(fonts.items[3].choice, Choice::Font("Georgia".into()));
        // The one in force is marked, whatever the case it was written in.
        fonts.mark("georgia");
        let marked: Vec<&str> = fonts
            .items
            .iter()
            .filter(|i| i.selected)
            .map(|i| i.label.as_str())
            .collect();
        assert_eq!(marked, ["Georgia"]);
        assert!(fonts.hit(0.0, 0.0).is_none());
        // No two choices overlap: each point picks at most one.
        for popup in [&sizes, &fonts] {
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
