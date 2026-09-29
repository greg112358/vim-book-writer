use crate::display::Display;
use crate::editor::Editor;
use crate::recent::Recent;
use crate::start::{self, StartAction, StartItem};
use crate::toolbar::{self, Click, Popup};
use crate::tree::Tree;
use crate::ui::{self, Chrome, Interface, Regions, Status};
use crate::vim::Key;
use std::path::{Path, PathBuf};

/// Which page of the app is showing.
#[derive(Debug, Clone, PartialEq)]
pub enum Screen {
    /// The page for choosing what to open, with a note about the last try that failed.
    Start {
        message: String,
    },
    Editing,
}

/// Something only the window can do, which the app asks for and the window then carries out and
/// reports back with `open_file` or `open_folder`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    Quit,
    PickFile,
    PickFolder,
}

/// Everything about the running app that isn't drawing: the screen, the editor, the recent files,
/// the open folder, and the toolbar's menus and hover. The window feeds it input and draws what
/// it says.
#[derive(Debug)]
pub struct App {
    pub screen: Screen,
    pub editor: Editor,
    pub recent: Recent,
    recent_path: Option<PathBuf>,
    pub tree: Option<Tree>,
    pub tree_visible: bool,
    /// The open toolbar menu.
    pub popup: Option<Popup>,
    /// The toolbar button, or on the start screen the item, the mouse is over.
    pub hover: Option<usize>,
    /// The color last chosen, shown on the color button.
    pub color: [u8; 3],
}

impl App {
    /// `recent_path` is where the recent list is saved when it changes, if anywhere.
    pub fn new(
        editor: Editor,
        screen: Screen,
        recent: Recent,
        recent_path: Option<PathBuf>,
    ) -> Self {
        Self {
            screen,
            editor,
            recent,
            recent_path,
            tree: None,
            tree_visible: false,
            popup: None,
            hover: None,
            color: [0xcc, 0, 0],
        }
    }

    pub fn regions(&self, size: (f32, f32)) -> Regions {
        ui::regions(size.0, size.1, self.tree_visible)
    }

    fn start_items(&self, size: (f32, f32)) -> Vec<StartItem> {
        start::layout(size, self.recent.files())
    }

    /// The window around the editor, or the whole start screen, as it should be drawn now.
    pub fn chrome(&mut self, size: (f32, f32)) -> Chrome {
        match &self.screen {
            Screen::Start { message } => {
                let items = self.start_items(size);
                let mut chrome = start::chrome(size, &items, self.hover);
                if !message.is_empty() {
                    chrome.labels.push(ui::Label {
                        text: message.clone(),
                        x: 24.0,
                        y: size.1 - 28.0,
                        size: 12.0,
                        color: [0xb0, 0x30, 0x30, 0xff],
                        bold: false,
                        italic: false,
                        align: ui::Align::Left,
                    });
                }
                chrome
            }
            Screen::Editing => {
                let regions = self.regions(size);
                let toolbar = toolbar::toolbar(regions.toolbar);
                let status = Status {
                    mode: self.editor.vim.mode,
                    file: self.editor.file_name(),
                    modified: self.editor.buf.is_modified(),
                    words: self.editor.word_count(),
                    message: self.editor.vim.message.clone(),
                };
                let interface = Interface {
                    toolbar: &toolbar,
                    hover: self.hover,
                    popup: self.popup.as_ref(),
                    color: self.color,
                };
                ui::chrome(&regions, &status, &interface)
            }
        }
    }

    /// Moves the mouse to (`x`, `y`). True if what is under it changed, so it needs a redraw.
    pub fn hover_at(&mut self, size: (f32, f32), x: f32, y: f32) -> bool {
        let hover = match self.screen {
            Screen::Start { .. } => start::hit(&self.start_items(size), x, y),
            Screen::Editing => toolbar::toolbar(self.regions(size).toolbar).hit(x, y),
        };
        std::mem::replace(&mut self.hover, hover) != hover
    }

    /// A left click at (`x`, `y`). In the text, `text_offset` is asked for the displayed offset
    /// under the mouse, given whether a block cursor is showing, since only the renderer knows
    /// where the text landed.
    pub fn click(
        &mut self,
        size: (f32, f32),
        (x, y): (f32, f32),
        text_offset: impl FnOnce(&Display, bool) -> Option<usize>,
    ) -> Vec<Effect> {
        if let Screen::Start { .. } = self.screen {
            let items = self.start_items(size);
            return match start::hit(&items, x, y) {
                Some(i) => self.start_action(items[i].action.clone()),
                None => Vec::new(),
            };
        }
        // An open menu takes the click: a choice applies, and anywhere else just closes it.
        if let Some(popup) = self.popup.take() {
            if let Some(item) = popup.hit(x, y) {
                if let Some(color) = item.swatch {
                    self.color = color;
                }
                self.editor.apply(&item.action);
            }
            return Vec::new();
        }
        let regions = self.regions(size);
        let toolbar = toolbar::toolbar(regions.toolbar);
        if let Some(i) = toolbar.hit(x, y) {
            match toolbar.buttons[i].click.clone() {
                Click::Do(action) => self.editor.apply(&action),
                Click::Open(menu) => {
                    let anchor = toolbar.buttons[i].bounds;
                    self.popup = Some(toolbar::popup(menu, anchor, size));
                }
            }
        } else if regions.editor.contains(x, y) {
            let block = self.editor.vim.mode != crate::vim::Mode::Insert;
            let offset = text_offset(self.editor.display().0, block);
            if let Some(offset) = offset {
                self.editor.click(offset);
            }
        }
        Vec::new()
    }

    fn start_action(&mut self, action: StartAction) -> Vec<Effect> {
        match action {
            StartAction::OpenFile => vec![Effect::PickFile],
            StartAction::OpenFolder => vec![Effect::PickFolder],
            StartAction::Open(path) => {
                self.open_file(&path);
                Vec::new()
            }
        }
    }

    /// A key press.
    pub fn key(&mut self, size: (f32, f32), key: Key) -> Vec<Effect> {
        if let Screen::Start { .. } = self.screen {
            return match key {
                Key::Ctrl('o') => vec![Effect::PickFile],
                Key::Char(c) => match start::key_action(c, &self.start_items(size)) {
                    Some(action) => self.start_action(action),
                    None => Vec::new(),
                },
                _ => Vec::new(),
            };
        }
        // Esc closes an open menu instead of reaching the editor.
        if key == Key::Esc && self.popup.take().is_some() {
            return Vec::new();
        }
        if key == Key::Ctrl('o') {
            if self.editor.buf.is_modified() {
                self.editor.vim.message = "E37: No write since last change".into();
                return Vec::new();
            }
            return vec![Effect::PickFile];
        }
        if self.editor.key(key) {
            vec![Effect::Quit]
        } else {
            Vec::new()
        }
    }

    /// Opens `path` for editing and puts it first in the recent files. If it can't be read the
    /// start screen says why.
    pub fn open_file(&mut self, path: &Path) {
        match Editor::open(path) {
            Ok(editor) => {
                self.editor = editor;
                self.screen = Screen::Editing;
                self.popup = None;
                self.hover = None;
                let absolute = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
                self.recent.add(absolute);
                if let Some(saved) = &self.recent_path {
                    // A recent list that won't save isn't worth interrupting writing for.
                    let _ = self.recent.save(saved);
                }
            }
            Err(error) => {
                self.screen = Screen::Start {
                    message: format!("Can't open {}: {error}", path.display()),
                };
            }
        }
    }

    /// Shows `folder`'s files in the tree on the right, ready to edit in a new document.
    pub fn open_folder(&mut self, folder: &Path) {
        self.tree = Some(Tree::new(folder));
        self.tree_visible = true;
        if let Screen::Start { .. } = self.screen {
            self.screen = Screen::Editing;
            self.hover = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vim::Mode;

    const SIZE: (f32, f32) = (1000.0, 700.0);

    fn temp_file(name: &str, text: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("bwa_app_{name}.dj"));
        std::fs::write(&path, text).unwrap();
        path
    }

    fn start_app(recent: Recent) -> App {
        App::new(
            Editor::default(),
            Screen::Start {
                message: String::new(),
            },
            recent,
            None,
        )
    }

    #[test]
    fn the_start_screen_asks_for_dialogs_and_opens_recent_files() {
        let path = temp_file("recent_a", "hello there");
        let mut recent = Recent::default();
        recent.add(path.clone());
        let mut app = start_app(recent);
        // Buttons and keys ask the window for a dialog.
        assert_eq!(app.key(SIZE, Key::Char('o')), [Effect::PickFile]);
        assert_eq!(app.key(SIZE, Key::Char('f')), [Effect::PickFolder]);
        assert_eq!(app.key(SIZE, Key::Ctrl('o')), [Effect::PickFile]);
        let items = start::layout(SIZE, app.recent.files());
        let click = |app: &mut App, item: &StartItem| {
            app.click(SIZE, (item.bounds.x + 4.0, item.bounds.y + 4.0), |_, _| {
                None
            })
        };
        assert_eq!(click(&mut app, &items[1]), [Effect::PickFolder]);
        assert_eq!(app.key(SIZE, Key::Char('x')), []);
        // Hovering lights an item, and reports only when it changes.
        assert!(app.hover_at(SIZE, items[0].bounds.x + 4.0, items[0].bounds.y + 4.0));
        assert!(!app.hover_at(SIZE, items[0].bounds.x + 5.0, items[0].bounds.y + 5.0));
        assert!(app.hover_at(SIZE, 1.0, 1.0));
        // A recent file opens on a click or its number key.
        assert_eq!(click(&mut app, &items[2]), []);
        assert_eq!(app.screen, Screen::Editing);
        assert_eq!(app.editor.buf.text(), "hello there");
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn opening_files_updates_and_saves_the_recent_list() {
        let dir = std::env::temp_dir().join("bwa_app_recent_list");
        let _ = std::fs::remove_dir_all(&dir);
        let list = dir.join("recent.toml");
        let (a, b) = (temp_file("list_a", "a"), temp_file("list_b", "b"));
        let mut app = App::new(
            Editor::default(),
            Screen::Start {
                message: String::new(),
            },
            Recent::default(),
            Some(list.clone()),
        );
        app.open_file(&a);
        app.open_file(&b);
        app.open_file(&a);
        let saved = Recent::load(&list);
        assert_eq!(saved.files().len(), 2);
        assert_eq!(saved, Recent::load(&list));
        assert!(saved.files()[0].ends_with("bwa_app_list_a.dj"));
        assert!(saved.files()[1].ends_with("bwa_app_list_b.dj"));
        for path in [a, b] {
            std::fs::remove_file(path).unwrap();
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_file_that_cant_be_opened_is_reported_on_the_start_screen() {
        let mut app = start_app(Recent::default());
        app.open_file(Path::new("/nonexistent/book.dj"));
        let Screen::Start { message } = app.screen.clone() else {
            panic!("left the start screen")
        };
        assert!(
            message.starts_with("Can't open /nonexistent/book.dj"),
            "{message}"
        );
        assert!(app.chrome(SIZE).labels.iter().any(|l| l.text == message));
        assert!(app.recent.files().is_empty());
    }

    #[test]
    fn opening_a_folder_shows_the_tree_and_leaves_the_start_screen() {
        let dir = std::env::temp_dir().join("bwa_app_folder");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("one.dj"), "x").unwrap();
        let mut app = start_app(Recent::default());
        app.open_folder(&dir);
        assert_eq!(app.screen, Screen::Editing);
        assert!(app.tree_visible);
        assert_eq!(app.tree.as_ref().unwrap().rows().len(), 1);
        assert!(app.regions(SIZE).tree.is_some());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn toolbar_buttons_and_menus_edit_the_text() {
        let mut app = App::new(Editor::default(), Screen::Editing, Recent::default(), None);
        for key in "ione two three\x1b0w".chars() {
            app.key(
                SIZE,
                if key == '\x1b' {
                    Key::Esc
                } else {
                    Key::Char(key)
                },
            );
        }
        let regions = app.regions(SIZE);
        let toolbar = toolbar::toolbar(regions.toolbar);
        let button = |label: &str| {
            let b = toolbar
                .buttons
                .iter()
                .find(|b| b.label == label)
                .unwrap()
                .bounds;
            (b.x + b.w / 2.0, b.y + b.h / 2.0)
        };
        // Bold: the word at the cursor.
        assert!(app.click(SIZE, button("B"), |_, _| None).is_empty());
        assert_eq!(app.editor.buf.text(), "one *two* three");
        // The color button opens a palette, which a choice applies and closes.
        app.click(SIZE, button("A"), |_, _| None);
        let popup = app.popup.clone().unwrap();
        let green = popup
            .items
            .iter()
            .find(|i| i.swatch == Some([0x2e, 0x8b, 0x57]))
            .unwrap();
        app.click(
            SIZE,
            (green.bounds.x + 3.0, green.bounds.y + 3.0),
            |_, _| None,
        );
        assert!(app.popup.is_none());
        assert_eq!(app.color, [0x2e, 0x8b, 0x57]);
        assert!(
            app.editor.buf.text().contains("color:#2e8b57"),
            "{}",
            app.editor.buf.text()
        );
        // Esc closes a menu without touching the editor; clicking away closes it and does nothing.
        app.click(SIZE, button("Font"), |_, _| None);
        assert!(app.popup.is_some());
        assert!(app.key(SIZE, Key::Esc).is_empty());
        assert!(app.popup.is_none());
        let before = app.editor.buf.text().to_string();
        app.click(SIZE, button("Size"), |_, _| None);
        app.click(SIZE, (500.0, 300.0), |_, _| {
            panic!("a click that closes a menu isn't a text click")
        });
        assert!(app.popup.is_none());
        assert_eq!(app.editor.buf.text(), before);
    }

    #[test]
    fn clicking_the_text_asks_where_and_puts_the_cursor_there() {
        let mut app = App::new(Editor::default(), Screen::Editing, Recent::default(), None);
        for key in "iabc def".chars() {
            app.key(SIZE, Key::Char(key));
        }
        app.key(SIZE, Key::Esc);
        let mut asked = None;
        let editor = app.regions(SIZE).editor;
        app.click(
            SIZE,
            (editor.x + 50.0, editor.y + 50.0),
            |display, block| {
                asked = Some((display.text.clone(), block));
                Some(5)
            },
        );
        assert_eq!(asked, Some(("abc def".to_string(), true)));
        assert_eq!(app.editor.buf.cursor(), 5);
        // Outside the editor, in the status bar, it doesn't ask.
        app.click(SIZE, (10.0, 695.0), |_, _| panic!("not in the text"));
    }

    #[test]
    fn ctrl_o_and_quitting() {
        let mut app = App::new(Editor::default(), Screen::Editing, Recent::default(), None);
        assert_eq!(app.key(SIZE, Key::Ctrl('o')), [Effect::PickFile]);
        for key in "ix".chars() {
            app.key(SIZE, Key::Char(key));
        }
        app.key(SIZE, Key::Esc);
        // With unsaved changes it won't leave them behind.
        assert!(app.key(SIZE, Key::Ctrl('o')).is_empty());
        assert!(app.editor.vim.message.starts_with("E37"));
        assert_eq!(app.editor.vim.mode, Mode::Normal);
        let mut quit = Vec::new();
        for c in ":q!\n".chars() {
            quit = app.key(SIZE, if c == '\n' { Key::Enter } else { Key::Char(c) });
        }
        assert_eq!(quit, [Effect::Quit]);
    }
}
