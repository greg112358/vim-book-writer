use crate::display::Display;
use crate::editor::Editor;
use crate::recent::Recent;
use crate::start::{self, StartAction, StartItem};
use crate::toolbar::{self, Click, Popup};
use crate::tree::Tree;
use crate::ui::{self, Chrome, Interface, Regions, Status, TreeRow, TreeView};
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

/// What has the keyboard: the text, or the file tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Editor,
    Tree,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PromptKind {
    Add,
    Rename,
    Copy,
    Move,
    ConfirmDelete,
}

/// A line the tree asks you to type, or to confirm, on the status bar.
#[derive(Debug, Clone, PartialEq)]
pub struct Prompt {
    pub label: String,
    pub text: String,
    kind: PromptKind,
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
    pub focus: Focus,
    pub prompt: Option<Prompt>,
    /// A note from the app, like why a file couldn't be moved, shown on the status bar.
    pub message: String,
    /// The first tree row showing, so the list scrolls only when the selection leaves it.
    tree_first: usize,
    /// The row and text of the last thing yanked in the tree, for the clipboard.
    pub yanked: Option<PathBuf>,
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
            focus: Focus::Editor,
            prompt: None,
            message: String::new(),
            tree_first: 0,
            yanked: None,
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
                let message = match &self.prompt {
                    Some(prompt) => format!("{}{}\u{258f}", prompt.label, prompt.text),
                    None if !self.message.is_empty() => self.message.clone(),
                    None => self.editor.vim.message.clone(),
                };
                let status = Status {
                    mode: self.editor.vim.mode,
                    file: self.editor.file_name(),
                    modified: self.editor.buf.is_modified(),
                    words: self.editor.word_count(),
                    message,
                };
                let tree = self.tree_view(regions.tree);
                let interface = Interface {
                    toolbar: &toolbar,
                    hover: self.hover,
                    popup: self.popup.as_ref(),
                    color: self.color,
                    tree: tree.as_ref(),
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
        if let Some(panel) = regions.tree.filter(|p| p.contains(x, y)) {
            self.focus = Focus::Tree;
            let row = ui::tree_row_at(panel, self.tree_first, x, y);
            if let (Some(row), Some(tree)) = (row, &mut self.tree) {
                tree.select(row);
                self.tree_key(Key::Enter);
            }
        } else if let Some(i) = toolbar.hit(x, y) {
            match toolbar.buttons[i].click.clone() {
                Click::Do(action) => self.editor.apply(&action),
                Click::Open(menu) => {
                    let anchor = toolbar.buttons[i].bounds;
                    self.popup = Some(toolbar::popup(menu, anchor, size));
                }
            }
        } else if regions.editor.contains(x, y) {
            self.focus = Focus::Editor;
            let block = self.editor.vim.mode != crate::vim::Mode::Insert;
            let offset = text_offset(self.editor.display().0, block);
            if let Some(offset) = offset {
                self.editor.click(offset);
            }
        }
        Vec::new()
    }

    /// The tree as its panel shows it: the rows from the first one showing, which moves only far
    /// enough to keep the selection in view.
    fn tree_view(&mut self, panel: Option<ui::Bounds>) -> Option<TreeView> {
        let (tree, panel) = (self.tree.as_ref()?, panel?);
        let capacity = ui::tree_capacity(panel).max(1);
        let selected = tree.selected_index();
        if selected < self.tree_first {
            self.tree_first = selected;
        } else if selected >= self.tree_first + capacity {
            self.tree_first = selected + 1 - capacity;
        }
        let rows = tree
            .rows()
            .iter()
            .enumerate()
            .skip(self.tree_first)
            .take(capacity)
            .map(|(i, row)| TreeRow {
                name: row
                    .path
                    .file_name()
                    .map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
                depth: row.depth,
                is_dir: row.is_dir,
                expanded: row.expanded,
                selected: i == selected,
            })
            .collect();
        let name = tree.root().file_name().map_or_else(
            || tree.root().display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        Some(TreeView {
            header: format!("{name}/"),
            rows,
            focused: self.focus == Focus::Tree,
        })
    }

    /// Opens the folder of the open file, or the current one, in the tree if it isn't showing.
    fn ensure_tree(&mut self) {
        if self.tree.is_none() {
            let folder = self
                .editor
                .path()
                .and_then(|p| std::fs::canonicalize(p).ok())
                .and_then(|p| p.parent().map(Path::to_path_buf))
                .or_else(|| std::env::current_dir().ok())
                .unwrap_or_else(|| PathBuf::from("."));
            self.tree = Some(Tree::new(folder));
        }
        self.tree_visible = true;
    }

    /// Opens a file chosen in the tree, unless that would throw away unsaved changes.
    fn open_from_tree(&mut self, path: &Path) -> bool {
        if self.editor.buf.is_modified() {
            self.message = "E37: No write since last change".into();
            return false;
        }
        self.open_file(path);
        true
    }

    fn start_prompt(&mut self, kind: PromptKind) {
        let Some(tree) = &self.tree else { return };
        let Some(row) = tree.selected() else {
            if kind == PromptKind::Add {
                self.prompt = Some(Prompt {
                    label: "Add (end with / for a folder): ".into(),
                    text: String::new(),
                    kind,
                });
            }
            return;
        };
        let name = row
            .path
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        let (label, text) = match kind {
            PromptKind::Add => ("Add (end with / for a folder): ".to_string(), String::new()),
            PromptKind::Rename => ("Rename to: ".into(), name),
            PromptKind::Copy => ("Copy to: ".into(), tree.relative(&row.path)),
            PromptKind::Move => ("Move to: ".into(), tree.relative(&row.path)),
            PromptKind::ConfirmDelete => (format!("Delete {name}? (y/n) "), String::new()),
        };
        self.prompt = Some(Prompt { label, text, kind });
    }

    /// A key while a prompt is open: typing, Backspace, Enter to carry it out, Esc to drop it.
    fn prompt_key(&mut self, key: Key) {
        let Some(prompt) = &mut self.prompt else {
            return;
        };
        if prompt.kind == PromptKind::ConfirmDelete {
            let confirmed = matches!(key, Key::Char('y' | 'Y'));
            self.prompt = None;
            if confirmed {
                self.finish_prompt(PromptKind::ConfirmDelete, String::new());
            }
            return;
        }
        match key {
            Key::Esc => self.prompt = None,
            Key::Enter => {
                let Prompt { kind, text, .. } = self.prompt.take().unwrap();
                self.finish_prompt(kind, text);
            }
            Key::Backspace => {
                prompt.text.pop();
            }
            Key::Char(c) => prompt.text.push(c),
            _ => {}
        }
    }

    fn finish_prompt(&mut self, kind: PromptKind, text: String) {
        let Some(tree) = &mut self.tree else { return };
        let selected = tree.selected().map(|row| row.path.clone());
        let open_path = self
            .editor
            .path()
            .and_then(|p| std::fs::canonicalize(p).ok());
        let is_open = selected
            .as_ref()
            .is_some_and(|s| Some(s) == open_path.as_ref());
        let result = match kind {
            PromptKind::Add => tree.add(&text),
            PromptKind::Rename => tree.rename(&text),
            PromptKind::Copy => tree.copy_to(&text),
            PromptKind::Move => tree.move_to(&text),
            PromptKind::ConfirmDelete => {
                tree.delete().map(|()| selected.clone().unwrap_or_default())
            }
        };
        match result {
            Ok(path) => {
                self.message.clear();
                match kind {
                    PromptKind::Add if path.is_file() => {
                        self.open_from_tree(&path);
                    }
                    // The open document follows its file when that is moved, or lets go of it
                    // when it is deleted, so saving doesn't write to the old place.
                    PromptKind::Rename | PromptKind::Move if is_open => {
                        self.editor.set_path(Some(path))
                    }
                    PromptKind::ConfirmDelete if is_open => self.editor.set_path(None),
                    _ => {}
                }
            }
            Err(error) => self.message = format!("{error}"),
        }
    }

    /// A key while the tree has the keyboard, using Neo-tree's: `j` `k` move, `a` add, `d` delete,
    /// `r` rename, `c` copy, `m` move, `y` yank, `p` paste, `H` hidden files, `R` refresh, Enter
    /// opens, Tab or `o` opens or expands without leaving the tree, and `q` closes it.
    fn tree_key(&mut self, key: Key) {
        let Some(tree) = &mut self.tree else {
            self.focus = Focus::Editor;
            return;
        };
        match key {
            Key::Char('j') | Key::Down => tree.move_selection(1),
            Key::Char('k') | Key::Up => tree.move_selection(-1),
            Key::Char('a') => self.start_prompt(PromptKind::Add),
            Key::Char('d') => self.start_prompt(PromptKind::ConfirmDelete),
            Key::Char('r') => self.start_prompt(PromptKind::Rename),
            Key::Char('c') => self.start_prompt(PromptKind::Copy),
            Key::Char('m') => self.start_prompt(PromptKind::Move),
            Key::Char('y') => {
                if let Some(path) = tree.yank() {
                    self.message = format!("Yanked {}", tree.relative(&path));
                    self.yanked = Some(path);
                }
            }
            Key::Char('p') => match tree.paste() {
                Ok(path) => self.message = format!("Pasted {}", tree.relative(&path)),
                Err(error) => self.message = format!("{error}"),
            },
            Key::Char('H') => tree.toggle_hidden(),
            Key::Char('R') => tree.refresh(),
            Key::Enter | Key::Char('\t' | 'o') => {
                let leave = key == Key::Enter;
                if let Some(path) = tree.activate()
                    && self.open_from_tree(&path)
                    && leave
                {
                    self.focus = Focus::Editor;
                }
            }
            Key::Char('q') => {
                self.tree_visible = false;
                self.focus = Focus::Editor;
            }
            Key::Esc => self.focus = Focus::Editor,
            _ => {}
        }
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
        self.message.clear();
        if self.prompt.is_some() {
            self.prompt_key(key);
            return Vec::new();
        }
        // Esc closes an open menu instead of reaching the editor.
        if key == Key::Esc && self.popup.take().is_some() {
            return Vec::new();
        }
        if key == Key::Ctrl('e') {
            // Show the tree and go to it; from there, go back to the text, and to it again.
            if !self.tree_visible || self.focus == Focus::Editor {
                self.ensure_tree();
                self.focus = Focus::Tree;
            } else {
                self.focus = Focus::Editor;
            }
            return Vec::new();
        }
        if self.focus == Focus::Tree {
            self.tree_key(key);
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
                let message = format!("Can't open {}: {error}", path.display());
                match self.screen {
                    Screen::Start { .. } => self.screen = Screen::Start { message },
                    Screen::Editing => self.message = message,
                }
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

    /// A folder with chapters/{one.dj,two.dj}, a.dj, and notes.dj, and an app editing a.dj.
    fn tree_app(name: &str) -> (App, PathBuf) {
        let root = std::env::temp_dir().join(format!("bwa_app_tree_{name}"));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("chapters")).unwrap();
        for file in ["chapters/one.dj", "chapters/two.dj", "a.dj", "notes.dj"] {
            std::fs::write(root.join(file), file).unwrap();
        }
        let root = std::fs::canonicalize(root).unwrap();
        let editor = Editor::open(&root.join("a.dj")).unwrap();
        (
            App::new(editor, Screen::Editing, Recent::default(), None),
            root,
        )
    }

    /// Types `keys`: `\n` is Enter, `\x1b` Esc, `\x08` Backspace, `\x05` Ctrl+E, and `\t` Tab.
    fn press(app: &mut App, keys: &str) {
        for c in keys.chars() {
            app.key(
                SIZE,
                match c {
                    '\n' => Key::Enter,
                    '\x1b' => Key::Esc,
                    '\x08' => Key::Backspace,
                    '\x05' => Key::Ctrl('e'),
                    c => Key::Char(c),
                },
            );
        }
    }

    fn rows(app: &App) -> Vec<String> {
        let tree = app.tree.as_ref().unwrap();
        tree.rows()
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let name = r.path.file_name().unwrap().to_string_lossy();
                let mark = if i == tree.selected_index() { "*" } else { "" };
                format!(
                    "{}{name}{}{mark}",
                    "  ".repeat(r.depth),
                    if r.is_dir { "/" } else { "" }
                )
            })
            .collect()
    }

    #[test]
    fn ctrl_e_shows_the_tree_and_passes_the_keyboard_back_and_forth() {
        let (mut app, root) = tree_app("focus");
        assert!(app.tree.is_none() && !app.tree_visible);
        press(&mut app, "\x05");
        assert!(app.tree_visible && app.focus == Focus::Tree);
        assert_eq!(app.tree.as_ref().unwrap().root(), root);
        // The tree has the keys now, so `x` doesn't reach the text.
        press(&mut app, "x");
        assert_eq!(app.editor.buf.text(), "a.dj");
        press(&mut app, "\x05");
        assert_eq!(app.focus, Focus::Editor);
        press(&mut app, "x");
        assert_eq!(app.editor.buf.text(), ".dj");
        press(&mut app, "\x05");
        assert_eq!(app.focus, Focus::Tree);
        // Esc gives the keyboard back; q closes the tree.
        press(&mut app, "\x1b");
        assert_eq!(app.focus, Focus::Editor);
        press(&mut app, "\x05q");
        assert!(!app.tree_visible);
        assert_eq!(app.focus, Focus::Editor);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_tree_moves_expands_and_opens_files() {
        let (mut app, root) = tree_app("open");
        press(&mut app, "\x05");
        assert_eq!(rows(&app), ["chapters/*", "a.dj", "notes.dj"]);
        press(&mut app, "\n");
        assert_eq!(
            rows(&app),
            ["chapters/*", "  one.dj", "  two.dj", "a.dj", "notes.dj"]
        );
        press(&mut app, "jj\n");
        // Enter on a file opens it and goes to the text; the tree keeps its place.
        assert_eq!(app.editor.file_name(), "two.dj");
        assert_eq!(app.editor.buf.text(), "chapters/two.dj");
        assert_eq!(app.focus, Focus::Editor);
        assert!(app.tree_visible);
        // Tab and `o` open without leaving the tree.
        press(&mut app, "\x05k\t");
        assert_eq!(app.editor.file_name(), "one.dj");
        assert_eq!(app.focus, Focus::Tree);
        press(&mut app, "j\t");
        assert_eq!(app.editor.file_name(), "two.dj");
        // Unsaved changes stay put: the file isn't replaced, and the tree says why.
        press(&mut app, "\x1bx\x05j\n");
        assert!(app.editor.buf.is_modified());
        assert_eq!(app.editor.file_name(), "two.dj");
        assert!(
            app.chrome(SIZE)
                .labels
                .iter()
                .any(|l| l.text.starts_with("E37"))
        );
        // The recent list picks up what was opened.
        assert_eq!(app.recent.files().len(), 2);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn add_rename_and_delete_go_through_prompts() {
        let (mut app, root) = tree_app("prompts");
        press(&mut app, "\x05j");
        // Add: the prompt shows on the status bar, a file is made and opened.
        press(&mut app, "a");
        assert!(
            app.chrome(SIZE)
                .labels
                .iter()
                .any(|l| l.text.starts_with("Add (end with / for a folder): "))
        );
        press(&mut app, "new.dj\n");
        assert!(root.join("new.dj").is_file());
        assert_eq!(app.editor.file_name(), "new.dj");
        assert!(app.prompt.is_none());
        // A name that ends in / is a folder, and Esc drops a prompt.
        press(&mut app, "a");
        press(&mut app, "more/\n");
        assert!(root.join("more").is_dir());
        press(&mut app, "aoops\x1b");
        assert!(app.prompt.is_none() && !root.join("oops").exists());
        // Rename starts with the current name; the open document follows its file.
        press(&mut app, "gg");
        while rows(&app)[app.tree.as_ref().unwrap().selected_index()] != "new.dj*" {
            press(&mut app, "j");
        }
        press(&mut app, "r");
        assert_eq!(app.prompt.as_ref().unwrap().text, "new.dj");
        press(&mut app, "\x08\x08\x08\x08\x08\x08\x08\x08moved.dj\n");
        assert!(root.join("moved.dj").is_file() && !root.join("new.dj").exists());
        assert!(app.editor.path().unwrap().ends_with("moved.dj"));
        // Delete asks first: n keeps it, y removes it, and the document lets go of the file.
        press(&mut app, "dn");
        assert!(root.join("moved.dj").exists());
        press(&mut app, "dy");
        assert!(!root.join("moved.dj").exists());
        assert!(app.editor.path().is_none());
        // Trouble is reported, not fatal.
        press(&mut app, "a../out.dj\n");
        assert!(!app.message.is_empty());
        assert!(!root.parent().unwrap().join("out.dj").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn yank_paste_copy_move_hidden_and_refresh() {
        let (mut app, root) = tree_app("files");
        std::fs::write(root.join(".secret.dj"), "s").unwrap();
        press(&mut app, "\x05j");
        assert_eq!(rows(&app)[1], "a.dj*");
        press(&mut app, "yp");
        assert!(root.join("a copy.dj").is_file());
        assert!(app.message.starts_with("Pasted"));
        // Copy and move start with the path, to be edited.
        press(&mut app, "c");
        assert_eq!(app.prompt.as_ref().unwrap().text, "a copy.dj");
        press(
            &mut app,
            "\x08\x08\x08\x08\x08\x08\x08\x08\x08\x08backup/b.dj\n",
        );
        assert!(root.join("backup/b.dj").is_file());
        press(&mut app, "m");
        assert_eq!(app.prompt.as_ref().unwrap().text, "backup/b.dj");
        press(&mut app, "\x08\x08\x08\x08c.dj\n");
        assert!(root.join("backup/c.dj").is_file() && !root.join("backup/b.dj").exists());
        // H shows dotfiles, R reads the folder again.
        assert!(!rows(&app).iter().any(|r| r.contains(".secret")));
        press(&mut app, "H");
        assert!(rows(&app).iter().any(|r| r.contains(".secret")));
        std::fs::write(root.join("late.dj"), "l").unwrap();
        assert!(!rows(&app).iter().any(|r| r.contains("late")));
        press(&mut app, "R");
        assert!(rows(&app).iter().any(|r| r.contains("late")));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn clicking_a_tree_row_selects_and_opens_it() {
        let (mut app, root) = tree_app("click");
        press(&mut app, "\x05");
        let panel = app.regions(SIZE).tree.unwrap();
        let row = |i: f32| {
            (
                panel.x + 30.0,
                panel.y + ui::TREE_HEADER + i * ui::TREE_ROW + 4.0,
            )
        };
        // The chapters folder opens; then the first file in it opens in the editor.
        app.click(SIZE, row(0.0), |_, _| None);
        assert_eq!(rows(&app).len(), 5);
        app.click(SIZE, row(1.0), |_, _| None);
        assert_eq!(app.editor.file_name(), "one.dj");
        assert_eq!(app.focus, Focus::Editor);
        // Clicking the text takes the keyboard back from the tree.
        press(&mut app, "\x05");
        assert_eq!(app.focus, Focus::Tree);
        let editor = app.regions(SIZE).editor;
        app.click(SIZE, (editor.x + 50.0, editor.y + 50.0), |_, _| Some(0));
        assert_eq!(app.focus, Focus::Editor);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_long_tree_scrolls_to_keep_the_selection_showing() {
        let (mut app, root) = tree_app("long");
        for i in 0..40 {
            std::fs::write(root.join(format!("f{i:02}.dj")), "x").unwrap();
        }
        press(&mut app, "\x05R");
        let visible = |app: &mut App| -> Vec<String> {
            let panel = app.regions(SIZE).tree;
            app.tree_view(panel)
                .unwrap()
                .rows
                .into_iter()
                .map(|r| r.name)
                .collect()
        };
        let capacity = ui::tree_capacity(app.regions(SIZE).tree.unwrap());
        assert_eq!(visible(&mut app).len(), capacity);
        assert_eq!(visible(&mut app)[0], "chapters");
        for _ in 0..30 {
            press(&mut app, "j");
        }
        let rows_now = visible(&mut app);
        assert_eq!(rows_now.len(), capacity);
        // The 31st row has scrolled into the last line showing.
        assert_eq!(rows_now.last().unwrap(), "f28.dj");
        std::fs::remove_dir_all(root).unwrap();
    }
}
