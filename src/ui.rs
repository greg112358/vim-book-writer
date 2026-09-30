use crate::diff::Change;
use crate::git::Tone;
use crate::toolbar::{Click, Popup, Toolbar};
use crate::vim::Mode;
use taffy::prelude::*;

/// The line height of the text, as a multiple of its size.
pub const LINE_HEIGHT: f32 = 1.5;

/// How the text is read: the font and size it is drawn in. This is a setting of the window, like
/// the light or dark look, and is not part of the document.
#[derive(Debug, Clone, PartialEq)]
pub struct Reading {
    pub family: String,
    /// In points.
    pub size_pt: f32,
}

impl Reading {
    /// The size of body text in pixels.
    pub fn size(&self) -> f32 {
        self.size_pt * 96.0 / 72.0
    }

    /// The height of a row of body text, which page scrolls count in.
    pub fn row(&self) -> f32 {
        self.size() * LINE_HEIGHT
    }
}

impl Default for Reading {
    fn default() -> Self {
        Self {
            family: "Serif".into(),
            size_pt: 14.0,
        }
    }
}

pub const TOOLBAR_HEIGHT: f32 = 42.0;
pub const STATUS_HEIGHT: f32 = 24.0;
pub const TREE_WIDTH: f32 = 300.0;
pub const OUTLINE_WIDTH: f32 = 240.0;

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
    /// The headings, on the left.
    pub outline: Option<Bounds>,
    pub editor: Bounds,
    pub tree: Option<Bounds>,
    pub status: Bounds,
}

/// Lays out a window `width` by `height`. The outline and tree take a fixed width and the editor
/// the rest.
pub fn regions(width: f32, height: f32, tree: bool, outline: bool) -> Regions {
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
    let outline_node = outline.then(|| taffy.new_leaf(fixed(OUTLINE_WIDTH, -1.0)).unwrap());
    let mut row: Vec<_> = outline_node.into_iter().collect();
    row.push(editor);
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
        outline: outline_node.map(|node| bounds(node, origin)),
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
/// The room for a row of buttons in the file tree, and the line of git news over the git ones.
pub const TREE_BUTTON_ROW: f32 = 30.0;
pub const GIT_LINE: f32 = 20.0;
/// Where the file tree's rows start, from the top of its panel: under two rows of file buttons,
/// and for a repository the branch line and two rows of git buttons.
pub const fn tree_top(git: bool) -> f32 {
    TREE_HEADER
        + 2.0 * TREE_BUTTON_ROW
        + 4.0
        + if git {
            GIT_LINE + 2.0 * TREE_BUTTON_ROW
        } else {
            0.0
        }
}
/// Where the file tree's rows start in a folder that is not a repository.
pub const TREE_TOP: f32 = tree_top(false);

/// The room a panel keeps at the bottom for its hint.
const FOOTER: f32 = 36.0;

/// How many rows fit in a panel whose rows start `top` down.
pub fn tree_capacity(panel: Bounds, top: f32) -> usize {
    ((panel.h - top - FOOTER) / TREE_ROW).floor().max(0.0) as usize
}

/// The row of a panel under (`x`, `y`), counting from `first`, the first row shown, when the rows
/// start `top` down.
pub fn tree_row_at(panel: Bounds, top: f32, first: usize, x: f32, y: f32) -> Option<usize> {
    let below = y - panel.y - top;
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
    /// What git says about it: how to color it, and the letter for a file.
    pub git: Option<(Tone, Option<char>)>,
}

/// What the file tree says about the repository it is in.
#[derive(Debug, Clone, PartialEq)]
pub struct GitInfo {
    pub branch: String,
    pub ahead: usize,
    pub behind: usize,
    pub changes: usize,
    pub staged: usize,
}

/// The file tree as it is now: the folder's name, the rows that fit from the first one showing,
/// and whether it has the keyboard.
#[derive(Debug, Clone, PartialEq)]
pub struct TreeView {
    pub header: String,
    pub rows: Vec<TreeRow>,
    pub focused: bool,
    pub git: Option<GitInfo>,
}

/// One heading in the outline.
#[derive(Debug, Clone, PartialEq)]
pub struct OutlineRow {
    pub title: String,
    pub level: usize,
    /// The section the cursor is in.
    pub current: bool,
    /// The row the outline's own cursor is on, when it has the keyboard.
    pub selected: bool,
}

/// The rows of the outline that fit, from the first one showing.
#[derive(Debug, Clone, PartialEq)]
pub struct OutlineView {
    pub rows: Vec<OutlineRow>,
    pub focused: bool,
}

/// A box in the middle of the window that takes the keyboard: a name to type, or a question.
#[derive(Debug, Clone, PartialEq)]
pub struct Dialog {
    pub title: String,
    /// What has been typed so far, for a dialog that asks for a name.
    pub input: Option<String>,
    /// The labels of the two buttons, for a dialog that asks a yes or no question.
    pub buttons: Option<(&'static str, &'static str)>,
    pub hint: &'static str,
}

const DIALOG_WIDTH: f32 = 440.0;
const DIALOG_HEIGHT: f32 = 132.0;

/// Where the dialog goes in a window `window` big.
pub fn dialog_bounds(window: (f32, f32)) -> Bounds {
    let w = DIALOG_WIDTH.min(window.0 - 24.0).max(0.0);
    Bounds {
        x: (window.0 - w) / 2.0,
        y: (window.1 * 0.3).max(40.0),
        w,
        h: DIALOG_HEIGHT,
    }
}

/// The yes and no buttons of a dialog.
pub fn dialog_buttons(dialog: Bounds) -> (Bounds, Bounds) {
    let (w, h) = (110.0, 28.0);
    let y = dialog.y + dialog.h - h - 14.0;
    let right = dialog.x + dialog.w - 16.0;
    (
        Bounds {
            x: right - 2.0 * w - 10.0,
            y,
            w,
            h,
        },
        Bounds {
            x: right - w,
            y,
            w,
            h,
        },
    )
}

/// The keys, by group, that the help box lists.
/// A group of keys in the help box: its title, and each key with what it does.
type HelpGroup = (&'static str, &'static [(&'static str, &'static str)]);

pub const HELP: [HelpGroup; 5] = [
    (
        "Panels",
        &[
            ("Ctrl-L", "file tree: go there, again to come back"),
            ("Ctrl-H", "outline: go there, again to come back"),
            ("j  k", "move up and down in a panel"),
            ("Enter", "open a file, fold a folder, go to a heading"),
            ("o  Tab", "the same, but stay in the panel"),
            ("Esc", "back to the text"),
            ("q", "hide the panel"),
        ],
    ),
    (
        "File tree",
        &[
            ("a  A", "new file, new folder"),
            ("g a  g u", "git: stage, unstage (g A: all)"),
            ("g r", "git: discard changes (asks first)"),
            ("g c", "git: commit"),
            ("g p  g l", "git: push, pull"),
            ("r", "rename"),
            ("d", "delete (asks first)"),
            ("c  m", "copy or move to a path you type"),
            ("y  p", "yank an item, paste a copy of it"),
            ("H  R", "show hidden files, refresh"),
        ],
    ),
    (
        "Moving",
        &[
            ("h j k l", "left, down, up, right"),
            ("w b e", "words (W B E: up to blanks)"),
            ("0 ^ $", "start, first letter, end of line"),
            ("gg  G  :N", "top, bottom, line N"),
            ("{  }", "paragraphs"),
            ("f F t T", "find a letter in the line (; , repeat)"),
            ("/ ? n N", "search (:noh clears the highlight)"),
            ("Ctrl-D  U", "half a page down, up"),
            ("Ctrl-F  B", "a page down, up"),
            ("zz zt zb", "line to middle, top, bottom"),
        ],
    ),
    (
        "Editing",
        &[
            ("i a I A o O", "start typing"),
            ("d c y", "with a motion; dd cc yy for lines"),
            ("x D C p P", "cut, cut to end, change to end, paste"),
            ("u  Ctrl-R  .", "undo, redo, repeat"),
            ("r  ~  J", "replace a letter, swap case, join lines"),
            ("v  V", "select (o: other end, p: paste over)"),
            ("\"a  \"+", "named register, the clipboard"),
        ],
    ),
    (
        "Files and text",
        &[
            ("Ctrl-S  :w", "save (:q  :wq  :q!  :e!)"),
            ("Ctrl-O", "open a file"),
            (
                "Ctrl-B  I",
                "bold, italic (Ctrl-B is page up in normal mode)",
            ),
            ("Ctrl-C X V", "copy, cut, paste"),
            ("F1  :help", "this list"),
        ],
    ),
];

/// The toolbar as it is now, for `chrome` to draw.
pub struct Interface<'a> {
    pub toolbar: &'a Toolbar,
    /// The button the mouse is over.
    pub hover: Option<usize>,
    pub popup: Option<&'a Popup>,
    pub tree: Option<&'a TreeView>,
    pub outline: Option<&'a OutlineView>,
    /// The file tree's buttons, and the one the mouse is over.
    pub tree_buttons: Option<&'a Toolbar>,
    pub tree_hover: Option<usize>,
    pub dialog: Option<&'a Dialog>,
    /// The list of keys is showing.
    pub help: bool,
    /// Whether auto-save is on, which the Auto button shows by staying pressed.
    pub autosave: bool,
    pub theme: &'a Theme,
    /// Whether the hovered button has waited long enough to show its tip.
    pub tip: bool,
}

/// Every color in the window, for one of the two looks.
#[derive(Debug, PartialEq)]
pub struct Theme {
    pub dark: bool,
    /// Behind the text, in linear RGB because it is the color the frame is cleared to.
    pub page: [f64; 3],
    pub bar: [u8; 4],
    pub panel: [u8; 4],
    pub border: [u8; 4],
    pub ink: [u8; 4],
    pub muted: [u8; 4],
    pub hover: [u8; 4],
    pub menu: [u8; 4],
    pub shadow: [u8; 4],
    /// A toggle button that is on.
    pub pressed: [u8; 4],
    pub tree_focused: [u8; 4],
    pub tree_unfocused: [u8; 4],
    pub tree_file: [u8; 4],
    pub key_hint: [u8; 4],
    pub error: [u8; 4],
    /// The start screen's lit recent file, and its lit button.
    pub start_row: [u8; 4],
    pub start_button: [u8; 4],
    pub text: [u8; 4],
    pub faint: [u8; 4],
    pub link: [u8; 4],
    pub quote: [u8; 4],
    pub selection: [u8; 4],
    /// Names in the file tree by what git says, and the bars in the margin by how a line changed.
    pub git_untracked: [u8; 4],
    pub git_added: [u8; 4],
    pub git_modified: [u8; 4],
    pub git_deleted: [u8; 4],
    pub git_conflict: [u8; 4],
    pub gutter_added: [u8; 4],
    pub gutter_modified: [u8; 4],
    pub gutter_deleted: [u8; 4],
    /// Searched-for text, and the match the cursor is on.
    pub found: [u8; 4],
    pub found_here: [u8; 4],
    pub block: [u8; 4],
    pub caret: [u8; 4],
}

const LIGHT: Theme = Theme {
    dark: false,
    page: [0.96, 0.94, 0.88],
    bar: [0xee, 0xe8, 0xda, 0xff],
    panel: [0xf2, 0xed, 0xe1, 0xff],
    border: [0xd8, 0xd0, 0xbd, 0xff],
    ink: [0x33, 0x33, 0x33, 0xff],
    muted: [0x7a, 0x74, 0x66, 0xff],
    hover: [0xe0, 0xd8, 0xc4, 0xff],
    menu: [0xfc, 0xf9, 0xf2, 0xff],
    shadow: [0, 0, 0, 0x28],
    pressed: [0xd2, 0xc8, 0xac, 0xff],
    tree_focused: [0xd9, 0xcf, 0xb4, 0xff],
    tree_unfocused: [0xe6, 0xdf, 0xcd, 0xff],
    tree_file: [0x44, 0x40, 0x38, 0xff],
    key_hint: [0x9a, 0x93, 0x84, 0xff],
    error: [0xb0, 0x30, 0x30, 0xff],
    start_row: [0xe8, 0xe1, 0xcf, 0xff],
    start_button: [0xdd, 0xd4, 0xbd, 0xff],
    text: [0x22, 0x22, 0x22, 0xff],
    faint: [0xb4, 0xab, 0x9a, 0xff],
    link: [0x1a, 0x5f, 0xb4, 0xff],
    quote: [0x5a, 0x5a, 0x5a, 0xff],
    selection: [0x7f, 0xae, 0xe6, 0x80],
    git_untracked: [0x58, 0x7c, 0x0c, 0xff],
    git_added: [0x38, 0x7a, 0x3c, 0xff],
    git_modified: [0x89, 0x55, 0x03, 0xff],
    git_deleted: [0xad, 0x07, 0x07, 0xff],
    git_conflict: [0xc0, 0x20, 0x30, 0xff],
    gutter_added: [0x2e, 0xa0, 0x43, 0xff],
    gutter_modified: [0x1b, 0x81, 0xa8, 0xff],
    gutter_deleted: [0xe0, 0x40, 0x40, 0xff],
    found: [0xf2, 0xd0, 0x5a, 0x90],
    found_here: [0xf0, 0x9a, 0x2c, 0xc0],
    block: [0x2b, 0x5c, 0xb8, 0x66],
    caret: [0x2b, 0x5c, 0xb8, 0xff],
};

const DARK: Theme = Theme {
    dark: true,
    page: [0.0116, 0.0110, 0.0097],
    bar: [0x26, 0x24, 0x21, 0xff],
    panel: [0x20, 0x1f, 0x1c, 0xff],
    border: [0x3a, 0x37, 0x33, 0xff],
    ink: [0xe2, 0xdd, 0xd2, 0xff],
    muted: [0x9a, 0x94, 0x86, 0xff],
    hover: [0x36, 0x33, 0x2e, 0xff],
    menu: [0x2b, 0x29, 0x25, 0xff],
    shadow: [0, 0, 0, 0x60],
    pressed: [0x48, 0x43, 0x3b, 0xff],
    tree_focused: [0x4a, 0x44, 0x38, 0xff],
    tree_unfocused: [0x36, 0x33, 0x2e, 0xff],
    tree_file: [0xcf, 0xc9, 0xbb, 0xff],
    key_hint: [0x7a, 0x74, 0x66, 0xff],
    error: [0xe0, 0x6c, 0x6c, 0xff],
    start_row: [0x30, 0x2e, 0x29, 0xff],
    start_button: [0x3d, 0x3a, 0x34, 0xff],
    text: [0xe6, 0xe1, 0xd6, 0xff],
    faint: [0x6b, 0x66, 0x5b, 0xff],
    link: [0x7f, 0xb0, 0xee, 0xff],
    quote: [0xb0, 0xaa, 0x9e, 0xff],
    selection: [0x3a, 0x5f, 0x94, 0xa0],
    git_untracked: [0x73, 0xc9, 0x91, 0xff],
    git_added: [0x81, 0xb8, 0x8b, 0xff],
    git_modified: [0xe2, 0xc0, 0x8d, 0xff],
    git_deleted: [0xc7, 0x4e, 0x39, 0xff],
    git_conflict: [0xe4, 0x67, 0x6b, 0xff],
    gutter_added: [0x58, 0xa6, 0x5d, 0xff],
    gutter_modified: [0x3f, 0x9c, 0xc4, 0xff],
    gutter_deleted: [0xd0, 0x50, 0x50, 0xff],
    found: [0x8a, 0x6d, 0x1c, 0x90],
    found_here: [0xc2, 0x84, 0x22, 0xc0],
    block: [0x7f, 0xae, 0xe6, 0x80],
    caret: [0x7f, 0xae, 0xe6, 0xff],
};

impl Theme {
    pub fn new(dark: bool) -> &'static Theme {
        if dark { &DARK } else { &LIGHT }
    }

    /// The color of a name in the file tree that git has something to say about.
    pub fn tone(&self, tone: Tone) -> [u8; 4] {
        match tone {
            Tone::Untracked => self.git_untracked,
            Tone::Added => self.git_added,
            Tone::Modified => self.git_modified,
            Tone::Deleted => self.git_deleted,
            Tone::Conflict => self.git_conflict,
        }
    }

    /// The color of the bar in the margin beside a changed line.
    pub fn gutter(&self, change: Change) -> [u8; 4] {
        match change {
            Change::Added => self.gutter_added,
            Change::Modified => self.gutter_modified,
            Change::Removed => self.gutter_deleted,
        }
    }
}

const LABEL_SIZE: f32 = 12.0;
const BUTTON_SIZE: f32 = 15.0;

/// What the status bar says.
#[derive(Debug, Clone, PartialEq)]
pub struct Status {
    pub mode: Mode,
    pub file: String,
    pub modified: bool,
    pub words: usize,
    /// The git branch and whether it has changes, when the file is in a repository.
    pub branch: Option<String>,
    /// Where the cursor is among them, or how many are selected.
    pub spot: Spot,
    /// A note from the editor, like a failed search, shown in place of the file.
    pub message: String,
}

/// Where the cursor is in the words of the text.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Spot {
    /// The word the cursor is on, counting from 1; 0 before the first.
    Word(usize),
    /// The number of words in the selection.
    Selected(usize),
}

fn mode_name(mode: Mode) -> &'static str {
    match mode {
        Mode::Normal => "NORMAL",
        Mode::Insert => "INSERT",
        Mode::Visual => "VISUAL",
        Mode::VisualLine => "V-LINE",
    }
}

/// A small box with `text` just under `anchor`, kept inside a window `width` wide.
fn push_tip(chrome: &mut Chrome, t: &Theme, width: f32, anchor: Bounds, text: &str) {
    let (size, pad) = (LABEL_SIZE, 6.0);
    // Text width isn't known here, so leave a little more than an average letter needs.
    let w = text.chars().count() as f32 * size * 0.56 + 2.0 * pad;
    let h = size * 1.3 + 2.0 * pad - 2.0;
    let x = anchor.x.min(width - w - 4.0).max(4.0);
    let y = anchor.y + anchor.h + 4.0;
    chrome
        .over_rects
        .push(Rect::new(Bounds { x, y, w, h }, t.border));
    chrome.over_rects.push(Rect::new(
        Bounds {
            x: x + 1.0,
            y: y + 1.0,
            w: w - 2.0,
            h: h - 2.0,
        },
        t.menu,
    ));
    chrome.over_labels.push(Label {
        text: text.into(),
        x: x + pad,
        y: y + pad - 1.0,
        size,
        color: t.ink,
        bold: false,
        italic: false,
        align: Align::Left,
    });
}

/// The toolbar and its buttons, the status bar and its text, the tree panel, and an open menu.
pub fn chrome(regions: &Regions, status: &Status, ui: &Interface<'_>) -> Chrome {
    let mut chrome = Chrome::default();
    let t = ui.theme;
    let line = |bounds: Bounds| Rect {
        h: 1.0,
        ..Rect::new(bounds, t.border)
    };

    let toolbar = regions.toolbar;
    chrome.rects.push(Rect::new(toolbar, t.bar));
    chrome.rects.push(Rect {
        y: toolbar.y + toolbar.h - 1.0,
        ..line(toolbar)
    });
    for (i, button) in ui.toolbar.buttons.iter().enumerate() {
        let b = button.bounds;
        let open =
            matches!((&button.click, ui.popup), (Click::Open(menu), Some(p)) if p.menu == *menu);
        let on = match button.click {
            Click::ToggleAutoSave => ui.autosave,
            Click::ToggleOutline => regions.outline.is_some(),
            _ => false,
        };
        if on {
            chrome.rects.push(Rect::new(b, t.pressed));
        } else if ui.hover == Some(i) || open {
            chrome.rects.push(Rect::new(b, t.hover));
        }
        chrome.labels.push(Label {
            // The theme button shows the look a click switches to.
            text: match button.click {
                Click::ToggleDark if t.dark => "\u{2600}".into(),
                _ => button.label.clone(),
            },
            x: b.x + b.w / 2.0,
            y: b.y + (b.h - BUTTON_SIZE * 1.3) / 2.0,
            size: BUTTON_SIZE,
            color: t.ink,
            bold: matches!(button.label.as_str(), "B" | "H1" | "H2" | "H3") || on,
            italic: button.label == "I",
            align: Align::Center,
        });
    }
    if let Some(popup) = ui.popup {
        let p = popup.bounds;
        chrome.over_rects.push(Rect {
            x: p.x + 2.0,
            y: p.y + 2.0,
            ..Rect::new(p, t.shadow)
        });
        chrome.over_rects.push(Rect::new(p, t.border));
        chrome.over_rects.push(Rect::new(
            Bounds {
                x: p.x + 1.0,
                y: p.y + 1.0,
                w: p.w - 2.0,
                h: p.h - 2.0,
            },
            t.menu,
        ));
        for item in &popup.items {
            let b = item.bounds;
            if item.selected {
                chrome.over_rects.push(Rect::new(b, t.pressed));
            }
            chrome.over_labels.push(Label {
                text: item.label.clone(),
                x: b.x + 6.0,
                y: b.y + (b.h - BUTTON_SIZE * 1.3) / 2.0,
                size: BUTTON_SIZE,
                color: t.ink,
                bold: item.selected,
                italic: false,
                align: Align::Left,
            });
        }
    }
    // The hovered button's tip goes under it, unless a menu is open there.
    let window = (regions.toolbar.w, regions.status.y + regions.status.h);
    if let (true, None) = (ui.tip, ui.popup) {
        let hovered = ui.hover.and_then(|i| ui.toolbar.buttons.get(i));
        let hovered_in_tree = ui
            .tree_hover
            .and_then(|i| ui.tree_buttons.and_then(|b| b.buttons.get(i)));
        if let Some(button) = hovered.or(hovered_in_tree) {
            push_tip(&mut chrome, t, window.0, button.bounds, &button.tip);
        }
    }

    if let Some(panel) = regions.outline {
        chrome.rects.push(Rect::new(panel, t.panel));
        chrome.rects.push(Rect {
            x: panel.x + panel.w - 1.0,
            w: 1.0,
            ..Rect::new(panel, t.border)
        });
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
            "Outline".into(),
            panel.x + 12.0,
            panel.y + 9.0,
            true,
            t.ink,
        ));
        chrome.rects.push(Rect {
            y: panel.y + TREE_HEADER - 2.0,
            w: panel.w - 1.0,
            ..line(panel)
        });
        match ui.outline {
            Some(view) if !view.rows.is_empty() => {
                for (i, row) in view.rows.iter().enumerate() {
                    let y = panel.y + TREE_HEADER + i as f32 * TREE_ROW;
                    if row.current || row.selected {
                        chrome.rects.push(Rect::new(
                            Bounds {
                                x: panel.x,
                                y,
                                w: panel.w - 1.0,
                                h: TREE_ROW,
                            },
                            if row.selected && view.focused {
                                t.tree_focused
                            } else {
                                t.tree_unfocused
                            },
                        ));
                    }
                    let x = panel.x + 12.0 + (row.level - 1) as f32 * 12.0;
                    // Text width isn't known here; cut long titles to what a row can hold.
                    let fit = ((panel.x + panel.w - 8.0 - x) / 6.8).max(1.0) as usize;
                    let title = if row.title.chars().count() > fit {
                        let cut: String = row.title.chars().take(fit.saturating_sub(1)).collect();
                        format!("{cut}\u{2026}")
                    } else {
                        row.title.clone()
                    };
                    let color = if row.level == 1 { t.ink } else { t.tree_file };
                    chrome.labels.push(text(
                        title,
                        x,
                        y + 3.0,
                        row.level == 1 || row.current,
                        color,
                    ));
                }
            }
            _ => chrome.labels.push(text(
                "No headings yet".into(),
                panel.x + 12.0,
                panel.y + TREE_HEADER + 3.0,
                false,
                t.muted,
            )),
        }
        let focused = ui.outline.is_some_and(|v| v.focused);
        let hint = if focused {
            [
                "j k move   Enter go to heading",
                "Esc back to the text   F1 help",
            ]
        } else {
            ["Ctrl-H to move around here", "F1 lists every key"]
        };
        for (i, line) in hint.into_iter().enumerate() {
            let mut label = text(
                line.into(),
                panel.x + 12.0,
                panel.y + panel.h - 34.0 + i as f32 * 15.0,
                false,
                t.muted,
            );
            label.size = 11.0;
            chrome.labels.push(label);
        }
    }

    if let Some(tree) = regions.tree {
        chrome.rects.push(Rect::new(tree, t.panel));
        chrome.rects.push(Rect {
            w: 1.0,
            ..Rect::new(tree, t.border)
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
                t.ink,
            ));
            chrome.rects.push(Rect {
                y: tree.y + TREE_HEADER - 2.0,
                x: tree.x + 1.0,
                w: tree.w - 1.0,
                ..line(tree)
            });
            for (i, row) in view.rows.iter().enumerate() {
                let y = tree.y
                    + tree_top(ui.tree.is_some_and(|v| v.git.is_some()))
                    + i as f32 * TREE_ROW;
                if row.selected {
                    let fill = if view.focused {
                        t.tree_focused
                    } else {
                        t.tree_unfocused
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
                let mut color = if row.is_dir { t.ink } else { t.tree_file };
                if let Some((tone, letter)) = row.git {
                    color = t.tone(tone);
                    // A file gets its letter at the right; a folder a dot, since it holds changes.
                    chrome.labels.push(Label {
                        align: Align::Right,
                        ..text(
                            letter.unwrap_or('\u{2022}').to_string(),
                            tree.x + tree.w - 12.0,
                            y + 3.0,
                            true,
                            color,
                        )
                    });
                }
                chrome.labels.push(text(
                    format!("{marker}{}", row.name),
                    tree.x + 10.0 + row.depth as f32 * 14.0,
                    y + 3.0,
                    row.is_dir,
                    color,
                ));
            }
        }
        // The branch, and how far it is from its upstream, over the git buttons.
        if let Some(git) = ui.tree.and_then(|v| v.git.as_ref()) {
            let mut news = git.branch.clone();
            if git.ahead > 0 {
                news.push_str(&format!(" \u{2191}{}", git.ahead));
            }
            if git.behind > 0 {
                news.push_str(&format!(" \u{2193}{}", git.behind));
            }
            news.push_str(&match (git.changes, git.staged) {
                (0, _) => " \u{b7} clean".to_string(),
                (n, 0) => format!(" \u{b7} {n} changed"),
                (n, s) => format!(" \u{b7} {n} changed, {s} staged"),
            });
            chrome.labels.push(Label {
                text: news,
                x: tree.x + 12.0,
                y: tree.y + TREE_HEADER + 2.0 * TREE_BUTTON_ROW + 2.0,
                size: 11.0,
                color: t.muted,
                bold: false,
                italic: false,
                align: Align::Left,
            });
        }
        if let Some(buttons) = ui.tree_buttons {
            for (i, button) in buttons.buttons.iter().enumerate() {
                let b = button.bounds;
                chrome.rects.push(Rect::new(
                    b,
                    if ui.tree_hover == Some(i) {
                        t.hover
                    } else {
                        t.bar
                    },
                ));
                chrome.labels.push(Label {
                    text: button.label.clone(),
                    x: b.x + b.w / 2.0,
                    y: b.y + (b.h - 13.0 * 1.3) / 2.0,
                    size: 13.0,
                    color: t.ink,
                    bold: false,
                    italic: false,
                    align: Align::Center,
                });
            }
        }
        let focused = ui.tree.is_some_and(|v| v.focused);
        let hint = if focused {
            [
                "j k move   Enter open / fold",
                "a A new   r rename   d delete",
            ]
        } else {
            ["Ctrl-L to move around here", "F1 lists every key"]
        };
        for (i, line) in hint.into_iter().enumerate() {
            chrome.labels.push(Label {
                text: line.into(),
                x: tree.x + 12.0,
                y: tree.y + tree.h - 34.0 + i as f32 * 15.0,
                size: 11.0,
                color: t.muted,
                bold: false,
                italic: false,
                align: Align::Left,
            });
        }
    }

    // A dialog dims the window and takes the middle of it.
    if let Some(dialog) = ui.dialog {
        let d = dialog_bounds(window);
        chrome.over_rects.push(Rect::new(
            Bounds {
                x: 0.0,
                y: 0.0,
                w: window.0,
                h: window.1,
            },
            [0, 0, 0, 0x50],
        ));
        chrome.over_rects.push(Rect::new(d, t.border));
        chrome.over_rects.push(Rect::new(
            Bounds {
                x: d.x + 1.0,
                y: d.y + 1.0,
                w: d.w - 2.0,
                h: d.h - 2.0,
            },
            t.menu,
        ));
        let label = |text: String, x: f32, y: f32, size: f32, bold: bool, color: [u8; 4]| Label {
            text,
            x,
            y,
            size,
            color,
            bold,
            italic: false,
            align: Align::Left,
        };
        chrome.over_labels.push(label(
            dialog.title.clone(),
            d.x + 16.0,
            d.y + 14.0,
            14.0,
            true,
            t.ink,
        ));
        if let Some(input) = &dialog.input {
            let field = Bounds {
                x: d.x + 16.0,
                y: d.y + 44.0,
                w: d.w - 32.0,
                h: 30.0,
            };
            chrome.over_rects.push(Rect::new(field, t.border));
            chrome.over_rects.push(Rect::new(
                Bounds {
                    x: field.x + 1.0,
                    y: field.y + 1.0,
                    w: field.w - 2.0,
                    h: field.h - 2.0,
                },
                t.panel,
            ));
            chrome.over_labels.push(label(
                format!("{input}\u{258f}"),
                field.x + 8.0,
                field.y + 7.0,
                14.0,
                false,
                t.ink,
            ));
        }
        if let Some((yes, no)) = dialog.buttons {
            for (b, text, fill) in [
                (dialog_buttons(d).0, yes, t.pressed),
                (dialog_buttons(d).1, no, t.hover),
            ] {
                chrome.over_rects.push(Rect::new(b, fill));
                chrome.over_labels.push(Label {
                    align: Align::Center,
                    ..label(text.into(), b.x + b.w / 2.0, b.y + 7.0, 13.0, true, t.ink)
                });
            }
        }
        chrome.over_labels.push(label(
            dialog.hint.into(),
            d.x + 16.0,
            d.y + if dialog.buttons.is_some() {
                50.0
            } else {
                d.h - 28.0
            },
            11.0,
            false,
            t.muted,
        ));
    }

    // The list of keys.
    if ui.help {
        let (pad, gap, line_h) = (20.0, 24.0, 17.0);
        let w = (window.0 - 32.0).clamp(0.0, 780.0);
        let column = (w - 2.0 * pad - gap) / 2.0;
        let columns: [&[HelpGroup]; 2] = [&HELP[..2], &HELP[2..]];
        let height = |sections: &[HelpGroup]| {
            sections
                .iter()
                .map(|(_, keys)| (keys.len() as f32 + 1.6) * line_h)
                .sum::<f32>()
        };
        let h =
            (height(columns[0]).max(height(columns[1])) + 2.0 * pad + 22.0).min(window.1 - 32.0);
        let b = Bounds {
            x: (window.0 - w) / 2.0,
            y: ((window.1 - h) / 2.0).max(16.0),
            w,
            h,
        };
        chrome.over_rects.push(Rect::new(
            Bounds {
                x: 0.0,
                y: 0.0,
                w: window.0,
                h: window.1,
            },
            [0, 0, 0, 0x50],
        ));
        chrome.over_rects.push(Rect::new(b, t.border));
        chrome.over_rects.push(Rect::new(
            Bounds {
                x: b.x + 1.0,
                y: b.y + 1.0,
                w: b.w - 2.0,
                h: b.h - 2.0,
            },
            t.menu,
        ));
        let label = |text: &str, x: f32, y: f32, bold: bool, color: [u8; 4]| Label {
            text: text.into(),
            x,
            y,
            size: 12.0,
            color,
            bold,
            italic: false,
            align: Align::Left,
        };
        for (c, sections) in columns.iter().enumerate() {
            let x = b.x + pad + c as f32 * (column + gap);
            let mut y = b.y + pad;
            for (title, keys) in *sections {
                chrome.over_labels.push(label(title, x, y, true, t.ink));
                y += line_h * 1.3;
                for (key, what) in *keys {
                    chrome.over_labels.push(label(key, x, y, true, t.tree_file));
                    chrome
                        .over_labels
                        .push(label(what, x + 104.0, y, false, t.muted));
                    y += line_h;
                }
                y += line_h * 0.3;
            }
        }
        chrome.over_labels.push(label(
            "Press any key to close",
            b.x + pad,
            b.y + b.h - pad - 4.0,
            false,
            t.muted,
        ));
    }

    let bar = regions.status;
    chrome.rects.push(Rect::new(bar, t.bar));
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
        t.ink,
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
        .push(label(file, bar.x + 90.0, Align::Left, false, t.ink));
    let total = format!(
        "{} {}",
        status.words,
        if status.words == 1 { "word" } else { "words" }
    );
    let words = match status.spot {
        Spot::Selected(n) if n > 0 => format!("{n} selected \u{b7} {total}"),
        Spot::Word(n) if n > 0 => format!("word {n} \u{b7} {total}"),
        _ => total,
    };
    let words = match &status.branch {
        Some(branch) => format!("{branch} \u{b7} {words}"),
        None => words,
    };
    chrome.labels.push(label(
        words,
        bar.x + bar.w - 12.0,
        Align::Right,
        false,
        t.muted,
    ));
    chrome
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_editor_fills_what_the_bars_and_tree_leave() {
        let r = regions(1000.0, 700.0, false, false);
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
        let r = regions(1000.0, 700.0, true, false);
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
            let r = regions(w, h, true, true);
            for b in [
                Some(r.toolbar),
                Some(r.editor),
                r.tree,
                r.outline,
                Some(r.status),
            ]
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
            h: TREE_HEADER + 10.0 * TREE_ROW + FOOTER,
        };
        assert_eq!(tree_capacity(panel, TREE_HEADER), 10);
        assert_eq!(tree_capacity(Bounds { h: 10.0, ..panel }, TREE_HEADER), 0);
        let y = |row: f32| panel.y + TREE_HEADER + row * TREE_ROW;
        assert_eq!(
            tree_row_at(panel, TREE_HEADER, 0, 710.0, y(0.0) + 1.0),
            Some(0)
        );
        assert_eq!(
            tree_row_at(panel, TREE_HEADER, 0, 710.0, y(3.0) + 5.0),
            Some(3)
        );
        // Counted from the first row that is showing, and not in the header or outside the panel.
        assert_eq!(
            tree_row_at(panel, TREE_HEADER, 7, 710.0, y(2.0) + 5.0),
            Some(9)
        );
        assert_eq!(
            tree_row_at(panel, TREE_HEADER, 0, 710.0, panel.y + 5.0),
            None
        );
        assert_eq!(tree_row_at(panel, TREE_HEADER, 0, 100.0, y(1.0)), None);
    }

    #[test]
    fn the_tree_panel_shows_its_header_rows_and_selection() {
        let regions = regions(1000.0, 700.0, true, false);
        let toolbar = crate::toolbar::Toolbar {
            buttons: Vec::new(),
        };
        let row = |name: &str, depth, is_dir, expanded, selected| TreeRow {
            name: name.into(),
            depth,
            is_dir,
            expanded,
            selected,
            git: None,
        };
        let view = TreeView {
            header: "novel".into(),
            rows: vec![
                row("chapters", 0, true, true, false),
                row("one.dj", 1, false, false, true),
                row("notes", 0, true, false, false),
            ],
            focused: true,
            git: None,
        };
        let status = Status {
            mode: Mode::Normal,
            file: "f".into(),
            modified: false,
            words: 0,
            branch: None,
            spot: Spot::Word(0),
            message: String::new(),
        };
        let interface = Interface {
            toolbar: &toolbar,
            hover: None,
            popup: None,
            tree: Some(&view),
            autosave: false,
            theme: Theme::new(false),
            tip: false,
            outline: None,
            tree_buttons: None,
            tree_hover: None,
            dialog: None,
            help: false,
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
        let regions = regions(800.0, 600.0, false, false);
        let status = |mode, modified, words, message: &str| Status {
            mode,
            file: "book.dj".into(),
            modified,
            words,
            branch: None,
            spot: Spot::Word(0),
            message: message.into(),
        };
        let toolbar = crate::toolbar::Toolbar {
            buttons: Vec::new(),
        };
        let interface = Interface {
            toolbar: &toolbar,
            hover: None,
            popup: None,
            tree: None,
            autosave: false,
            theme: Theme::new(false),
            tip: false,
            outline: None,
            tree_buttons: None,
            tree_hover: None,
            dialog: None,
            help: false,
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
        // Where the cursor is, or how much is selected, goes before the total.
        for (spot, want) in [
            (Spot::Word(12), "word 12 \u{b7} 1234 words"),
            (Spot::Selected(5), "5 selected \u{b7} 1234 words"),
            (Spot::Selected(0), "1234 words"),
        ] {
            let s = Status {
                spot,
                ..status(Mode::Normal, false, 1234, "")
            };
            assert_eq!(texts(s)[2], want);
        }
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
            tree: None,
            autosave: false,
            theme: Theme::new(false),
            tip: false,
            outline: None,
            tree_buttons: None,
            tree_hover: None,
            dialog: None,
            help: false,
        };
        for label in chrome(&regions, &status(Mode::Normal, false, 3, ""), &interface).labels {
            assert!(
                label.y >= bar.y && label.y + label.size <= bar.y + bar.h,
                "{label:?}"
            );
        }
    }
}
