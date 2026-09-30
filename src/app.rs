use crate::autosave::AutoSave;
use crate::config::Config;
use crate::display::Display;
use crate::editor::Editor;
use crate::git::{Repo, Status as GitStatus};
use crate::outline;
use crate::recent::Recent;
use crate::start::{self, StartAction, StartItem};
use crate::toolbar::{self, Choice, Click, Popup};
use crate::tree::Tree;
use crate::ui::{
    self, Chrome, Dialog, GitInfo, Interface, OutlineRow, OutlineView, Reading, Regions, Status,
    Theme, TreeRow, TreeView,
};
use crate::vim::{Key, ViewMove};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

/// Which page of the app is showing.
#[derive(Debug, Clone, PartialEq)]
pub enum Screen {
    /// The page for choosing what to open, with a note about the last try that failed.
    Start {
        message: String,
    },
    Editing,
}

/// How long the text must sit unchanged before auto-save writes it.
const AUTOSAVE_DELAY: Duration = Duration::from_millis(2000);
/// How long the mouse rests on a toolbar button before its tip shows.
const TIP_DELAY: Duration = Duration::from_millis(500);

/// What has the keyboard: the text, the file tree, or the outline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Editor,
    Tree,
    Outline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PromptKind {
    Add,
    AddFolder,
    Rename,
    Copy,
    Move,
    ConfirmDelete,
    /// Throw away the changes git shows in a file or folder.
    ConfirmDiscard,
    /// The message for a commit.
    Commit,
}

impl PromptKind {
    /// Asks a question to answer yes or no, rather than a name to type.
    fn confirms(self) -> bool {
        matches!(self, Self::ConfirmDelete | Self::ConfirmDiscard)
    }
}

/// What git says about the folder in the tree, or the folder of the open file.
#[derive(Debug)]
struct GitState {
    repo: Repo,
    status: GitStatus,
    /// The folder this was worked out for, as a full path: names in the tree are from it.
    folder: PathBuf,
}

/// A git command running in the background, and what it is doing.
#[derive(Debug)]
struct Job {
    label: &'static str,
    done: Receiver<Result<String, String>>,
}

/// A name the tree asks you to type, or a question to answer, in a dialog.
#[derive(Debug, Clone, PartialEq)]
pub struct Prompt {
    pub label: String,
    pub text: String,
    kind: PromptKind,
}

/// A file format to export the document as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportKind {
    Markdown,
    Pdf,
}

/// What the text from the system clipboard is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardUse {
    /// The `"+` register, which a vim command is waiting to read.
    Register,
    /// Ctrl+V.
    Paste,
}

/// Something only the window can do, which the app asks for and the window then carries out and
/// reports back with `open_file` or `open_folder`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    Quit,
    PickFile,
    PickFolder,
    /// Ask where to put an export; the window then calls `export_to`.
    PickExport(ExportKind),
    /// Put this text on the system clipboard.
    SetClipboard(String),
    /// Read the system clipboard; the window then calls `clipboard_text` with what it holds.
    ReadClipboard(ClipboardUse),
    /// Scroll the view by this many rows of text, down if positive.
    ScrollRows(i32),
    /// Put the cursor's line in the middle, at the top or at the bottom of the view.
    Align(ViewMove),
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
    pub focus: Focus,
    /// The font and size the text is read in, from the settings.
    pub reading: Reading,
    pub prompt: Option<Prompt>,
    /// A note from the app, like why a file couldn't be moved, shown on the status bar.
    pub message: String,
    /// The first tree row showing, so the list scrolls only when the selection leaves it.
    tree_first: usize,
    /// The row and text of the last thing yanked in the tree, for the clipboard.
    pub yanked: Option<PathBuf>,
    pub autosave: AutoSave,
    config: Config,
    config_path: Option<PathBuf>,
    /// When the mouse came to rest on the hovered button, and whether its tip is showing yet.
    hover_since: Instant,
    pub tip_shown: bool,
    /// Counts the files opened, so the window can start a new one at the top.
    pub opened: u64,
    /// The first outline row showing, so the list scrolls only when the current heading leaves it.
    outline_first: usize,
    /// The heading the outline's own cursor is on, while it has the keyboard.
    outline_sel: usize,
    /// The list of keys is showing.
    pub help: bool,
    /// The file tree button the mouse is over.
    tree_hover: Option<usize>,
    git: Option<GitState>,
    /// A `g` was typed in the tree and the next key says which git command.
    tree_g: bool,
    job: Option<Job>,
    /// The editor's count of saves as of the last look at git.
    saves_seen: u64,
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
            focus: Focus::Editor,
            reading: Reading::default(),
            prompt: None,
            message: String::new(),
            tree_first: 0,
            yanked: None,
            autosave: AutoSave::new(AUTOSAVE_DELAY, Instant::now()),
            config: Config::default(),
            config_path: None,
            hover_since: Instant::now(),
            tip_shown: false,
            opened: 0,
            outline_first: 0,
            outline_sel: 0,
            help: false,
            tree_hover: None,
            git: None,
            tree_g: false,
            job: None,
            saves_seen: 0,
        }
    }

    /// Applies the saved settings, and keeps the file to save them to when one changes.
    pub fn set_config(&mut self, config: Config, path: Option<PathBuf>) {
        self.autosave.enabled = config.auto_save;
        self.reading = Reading {
            family: config.font.clone(),
            size_pt: config.size,
        };
        self.config = config;
        self.config_path = path;
    }

    /// Call after input and when `next_wake` comes: saves the file if auto-save is on and the text
    /// has been still for long enough.
    pub fn tick(&mut self, now: Instant) -> bool {
        let mut changed = false;
        if self.hovering() && !self.tip_shown && now >= self.hover_since + TIP_DELAY {
            self.tip_shown = true;
            changed = true;
        }
        let due = self.autosave.poll(now, &self.editor.buf);
        if due && self.screen == Screen::Editing && self.editor.path().is_some() {
            self.editor.save();
        }
        // A push or pull that has finished says how it went.
        if let Some(job) = &self.job {
            let result = match job.done.try_recv() {
                Ok(result) => Some(result),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => Some(Err("git stopped".into())),
            };
            if let Some(result) = result {
                let label = job.label;
                self.job = None;
                self.message = match result {
                    Ok(said) => format!("{label} done: {said}"),
                    Err(error) => format!("{label} failed: {error}"),
                };
                self.refresh_git();
                changed = true;
            }
        }
        self.look_after_save();
        changed
    }

    fn hovering(&self) -> bool {
        self.hover.is_some() || self.tree_hover.is_some()
    }

    /// When `tick` next has something to do, for the window to wake at.
    pub fn next_wake(&self) -> Option<Instant> {
        let tip = (self.hovering() && !self.tip_shown && self.screen == Screen::Editing)
            .then(|| self.hover_since + TIP_DELAY);
        let job = self
            .job
            .as_ref()
            .map(|_| Instant::now() + Duration::from_millis(200));
        [self.autosave.next_deadline(), tip, job]
            .into_iter()
            .flatten()
            .min()
    }

    /// The colors to draw with.
    pub fn theme(&self) -> &'static Theme {
        Theme::new(self.config.dark)
    }

    pub fn regions(&self, size: (f32, f32)) -> Regions {
        ui::regions(size.0, size.1, self.tree_visible, self.config.outline)
    }

    fn start_items(&self, size: (f32, f32)) -> Vec<StartItem> {
        start::layout(size, self.recent.files())
    }

    /// The window around the editor, or the whole start screen, as it should be drawn now.
    pub fn chrome(&mut self, size: (f32, f32)) -> Chrome {
        match &self.screen {
            Screen::Start { message } => {
                let items = self.start_items(size);
                let mut chrome = start::chrome(size, &items, self.hover, self.theme());
                if !message.is_empty() {
                    chrome.labels.push(ui::Label {
                        text: message.clone(),
                        x: 24.0,
                        y: size.1 - 28.0,
                        size: 12.0,
                        color: self.theme().error,
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
                // What is being typed comes first: the `:` and `/` line.
                let message = match self.editor.vim.command_line() {
                    Some((prompt, line)) => format!("{prompt}{line}\u{258f}"),
                    None if !self.message.is_empty() => self.message.clone(),
                    None => self.editor.vim.message.clone(),
                };
                let branch = self.git.as_ref().map(|git| {
                    let s = &git.status;
                    let mut branch = s.branch.clone();
                    branch.push_str(if s.files.is_empty() { "" } else { "*" });
                    if s.ahead > 0 {
                        branch.push_str(&format!(" \u{2191}{}", s.ahead));
                    }
                    if s.behind > 0 {
                        branch.push_str(&format!(" \u{2193}{}", s.behind));
                    }
                    branch
                });
                let status = Status {
                    branch,
                    mode: self.editor.vim.mode,
                    file: self.editor.file_name(),
                    modified: self.editor.buf.is_modified(),
                    words: self.editor.word_count(),
                    spot: self.editor.spot(),
                    message,
                };
                let tree = self.tree_view(regions.tree);
                let outline = self.outline_view(regions.outline);
                let in_repo = self.git.is_some();
                let tree_buttons = regions.tree.map(|p| toolbar::tree_buttons(p, in_repo));
                let staged = self.git.as_ref().map_or(0, |g| g.status.staged());
                let dialog = self.prompt.as_ref().map(|prompt| match prompt.kind {
                    PromptKind::ConfirmDelete => Dialog {
                        title: prompt.label.clone(),
                        input: None,
                        buttons: Some(("Delete (y)", "Keep it (n)")),
                        hint: "This can't be undone.",
                    },
                    PromptKind::ConfirmDiscard => Dialog {
                        title: prompt.label.clone(),
                        input: None,
                        buttons: Some(("Discard (y)", "Keep changes (n)")),
                        hint: "This can't be undone.",
                    },
                    PromptKind::Commit => Dialog {
                        title: prompt.label.trim_end().to_string(),
                        input: Some(prompt.text.clone()),
                        buttons: None,
                        hint: if staged > 0 {
                            "Enter to commit what is staged   Esc to cancel"
                        } else {
                            "Nothing is staged, so Enter commits every change   Esc to cancel"
                        },
                    },
                    _ => Dialog {
                        title: prompt.label.trim_end().to_string(),
                        input: Some(prompt.text.clone()),
                        buttons: None,
                        hint: "Enter to confirm   Esc to cancel",
                    },
                });
                let interface = Interface {
                    outline: outline.as_ref(),
                    tree_buttons: tree_buttons.as_ref(),
                    tree_hover: self.tree_hover,
                    dialog: dialog.as_ref(),
                    help: self.help,
                    toolbar: &toolbar,
                    hover: self.hover,
                    popup: self.popup.as_ref(),
                    tree: tree.as_ref(),
                    autosave: self.autosave.enabled,
                    theme: self.theme(),
                    tip: self.tip_shown,
                };
                ui::chrome(&regions, &status, &interface)
            }
        }
    }

    /// Moves the mouse to (`x`, `y`). True if what is under it changed, so it needs a redraw.
    pub fn hover_at(&mut self, size: (f32, f32), x: f32, y: f32) -> bool {
        let in_repo = self.git.is_some();
        let (hover, tree_hover) = match self.screen {
            Screen::Start { .. } => (start::hit(&self.start_items(size), x, y), None),
            Screen::Editing => {
                let regions = self.regions(size);
                (
                    toolbar::toolbar(regions.toolbar).hit(x, y),
                    regions
                        .tree
                        .and_then(|p| toolbar::tree_buttons(p, in_repo).hit(x, y)),
                )
            }
        };
        let changed = std::mem::replace(&mut self.hover, hover) != hover
            || std::mem::replace(&mut self.tree_hover, tree_hover) != tree_hover;
        if changed {
            self.hover_since = Instant::now();
            self.tip_shown = false;
        }
        changed
    }

    /// A left click at (`x`, `y`). In the text, `text_offset` is asked for the displayed offset
    /// under the mouse, given whether a block cursor is showing, since only the renderer knows
    /// where the text landed.
    pub fn click(
        &mut self,
        size: (f32, f32),
        at: (f32, f32),
        text_offset: impl FnOnce(&Display, bool) -> Option<usize>,
    ) -> Vec<Effect> {
        let effects = self.clicked(size, at, text_offset);
        self.look_after_save();
        effects
    }

    fn clicked(
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
        // A click leaves the `:` or `/` line, whose preview no longer matches what is shown.
        self.editor.vim.close_command_line();
        // A dialog or the list of keys is in the way of everything else.
        if let Some(prompt) = &self.prompt {
            if prompt.kind.confirms() {
                let dialog = ui::dialog_bounds(size);
                let (yes, no) = ui::dialog_buttons(dialog);
                if yes.contains(x, y) {
                    self.prompt_key(Key::Char('y'));
                } else if no.contains(x, y) || !dialog.contains(x, y) {
                    self.prompt_key(Key::Char('n'));
                }
            }
            return Vec::new();
        }
        if self.help {
            self.help = false;
            return Vec::new();
        }
        // An open menu takes the click: a choice applies, and anywhere else just closes it.
        if let Some(popup) = self.popup.take() {
            if let Some(item) = popup.hit(x, y) {
                self.choose(&item.choice);
            }
            return Vec::new();
        }
        // A click answers the tip.
        self.hover_since = Instant::now();
        self.tip_shown = false;
        let regions = self.regions(size);
        let toolbar = toolbar::toolbar(regions.toolbar);
        if let Some(panel) = regions.outline.filter(|p| p.contains(x, y)) {
            // A heading takes the cursor there and the view to it.
            let row = ui::tree_row_at(panel, ui::TREE_HEADER, self.outline_first, x, y);
            let offset = row.and_then(|i| self.editor.headings().get(i).map(|h| h.offset));
            if let Some(offset) = offset {
                self.focus = Focus::Editor;
                self.editor.goto(offset);
                return vec![Effect::Align(ViewMove::Top)];
            }
        } else if let Some(panel) = regions.tree.filter(|p| p.contains(x, y)) {
            self.focus = Focus::Tree;
            let git = self.git.is_some();
            let row = ui::tree_row_at(panel, ui::tree_top(git), self.tree_first, x, y);
            let buttons = toolbar::tree_buttons(panel, git);
            let button = buttons.hit(x, y).map(|i| buttons.buttons[i].click.clone());
            if let Some(Click::TreeKey(key)) = button {
                self.tree_key(Key::Char(key));
            } else if let Some(Click::TreeGit(key)) = button {
                self.tree_key(Key::Char('g'));
                self.tree_key(Key::Char(key));
            } else if let (Some(row), Some(tree)) = (row, &mut self.tree) {
                tree.select(row);
                self.tree_key(Key::Enter);
            }
        } else if let Some(i) = toolbar.hit(x, y) {
            match toolbar.buttons[i].click.clone() {
                Click::Do(action) => self.editor.apply(&action),
                Click::Open(menu) => self.open_menu(menu, toolbar.buttons[i].bounds, size),
                Click::Save => {
                    self.editor.save();
                }
                Click::ExportMarkdown => return vec![Effect::PickExport(ExportKind::Markdown)],
                Click::ExportPdf => return vec![Effect::PickExport(ExportKind::Pdf)],
                click @ (Click::ToggleAutoSave | Click::ToggleDark | Click::ToggleOutline) => {
                    self.toggle_setting(&click)
                }
                Click::Help => self.help = true,
                Click::TreeKey(_) | Click::TreeGit(_) => {}
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

    /// Opens the list under a toolbar button, with what is in force marked.
    pub fn open_menu(&mut self, menu: toolbar::Menu, anchor: ui::Bounds, size: (f32, f32)) {
        let mut popup = toolbar::popup(menu, anchor, size);
        popup.mark(&match menu {
            toolbar::Menu::Font => self.config.font.clone(),
            toolbar::Menu::Size => format!("{}", self.config.size),
        });
        self.popup = Some(popup);
    }

    /// Reads the text in the font or size chosen from a menu, and remembers it in the settings
    /// file. The document itself is not touched.
    fn choose(&mut self, choice: &Choice) {
        match choice {
            Choice::Font(name) => self.config.font.clone_from(name),
            Choice::Size(pt) => self.config.size = *pt,
        }
        self.reading = Reading {
            family: self.config.font.clone(),
            size_pt: self.config.size,
        };
        if let Some(path) = &self.config_path {
            // A setting that won't save still works until the app closes.
            let _ = self.config.save(path);
        }
    }

    /// Flips a setting the toolbar has a button for, and remembers it in the settings file.
    fn toggle_setting(&mut self, which: &Click) {
        match which {
            Click::ToggleDark => self.config.dark = !self.config.dark,
            Click::ToggleOutline => self.config.outline = !self.config.outline,
            _ => {
                self.autosave.enabled = !self.autosave.enabled;
                self.config.auto_save = self.autosave.enabled;
            }
        }
        if let Some(path) = &self.config_path {
            // A setting that won't save still works until the app closes.
            let _ = self.config.save(path);
        }
    }

    /// The outline as its panel shows it: the rows from the first one showing, which moves only
    /// far enough to keep the heading the cursor is under in view.
    fn outline_view(&mut self, panel: Option<ui::Bounds>) -> Option<OutlineView> {
        let panel = panel?;
        let cursor = self.editor.buf.cursor();
        let focused = self.focus == Focus::Outline;
        let headings = self.editor.headings();
        let current = outline::current(headings, cursor);
        let capacity = ui::tree_capacity(panel, ui::TREE_HEADER).max(1);
        self.outline_sel = self.outline_sel.min(headings.len().saturating_sub(1));
        // Keep in view the row that has the cursor: the outline's own while it has the keyboard.
        if let Some(anchor) = if focused {
            Some(self.outline_sel)
        } else {
            current
        } {
            if anchor < self.outline_first {
                self.outline_first = anchor;
            } else if anchor >= self.outline_first + capacity {
                self.outline_first = anchor + 1 - capacity;
            }
        }
        self.outline_first = self.outline_first.min(headings.len().saturating_sub(1));
        let rows = headings
            .iter()
            .enumerate()
            .skip(self.outline_first)
            .take(capacity)
            .map(|(i, h)| OutlineRow {
                title: h.title.clone(),
                level: h.level,
                current: Some(i) == current,
                selected: focused && i == self.outline_sel,
            })
            .collect();
        Some(OutlineView { rows, focused })
    }

    /// The tree as its panel shows it: the rows from the first one showing, which moves only far
    /// enough to keep the selection in view.
    fn tree_view(&mut self, panel: Option<ui::Bounds>) -> Option<TreeView> {
        let (tree, panel) = (self.tree.as_ref()?, panel?);
        let capacity = ui::tree_capacity(panel, ui::tree_top(self.git.is_some())).max(1);
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
                git: self.git.as_ref().and_then(|git| {
                    // Names in the tree are from the folder as given; the status is from the real one.
                    let path = git.folder.join(row.path.strip_prefix(tree.root()).ok()?);
                    if row.is_dir {
                        git.status.dirs.get(&path).map(|&tone| (tone, None))
                    } else {
                        git.status
                            .files
                            .get(&path)
                            .map(|e| (e.tone(), Some(e.letter())))
                    }
                }),
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
            git: self.git.as_ref().map(|git| GitInfo {
                branch: git.status.branch.clone(),
                ahead: git.status.ahead,
                behind: git.status.behind,
                changes: git.status.files.len(),
                staged: git.status.staged(),
            }),
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
            if matches!(kind, PromptKind::Add | PromptKind::AddFolder) {
                self.prompt = Some(Prompt {
                    label: "New file: ".into(),
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
            PromptKind::Add => ("New file: ".to_string(), String::new()),
            PromptKind::AddFolder => ("New folder: ".to_string(), String::new()),
            PromptKind::Rename => ("Rename to: ".into(), name),
            PromptKind::Copy => ("Copy to: ".into(), tree.relative(&row.path)),
            PromptKind::Move => ("Move to: ".into(), tree.relative(&row.path)),
            PromptKind::ConfirmDelete => (format!("Delete \u{201c}{name}\u{201d}?"), String::new()),
            // Only the git commands start these two, which know what to put in them.
            PromptKind::ConfirmDiscard | PromptKind::Commit => return,
        };
        self.prompt = Some(Prompt { label, text, kind });
    }

    /// A key while a prompt is open: typing, Backspace, Enter to carry it out, Esc to drop it.
    fn prompt_key(&mut self, key: Key) {
        let Some(prompt) = &mut self.prompt else {
            return;
        };
        if prompt.kind.confirms() {
            let confirmed = matches!(key, Key::Char('y' | 'Y'));
            let Prompt { kind, text, .. } = self.prompt.take().unwrap();
            if confirmed {
                self.finish_prompt(kind, text);
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
        match kind {
            PromptKind::Commit => return self.git_commit(&text),
            PromptKind::ConfirmDiscard => return self.git_discard(&PathBuf::from(text)),
            _ => {}
        }
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
            PromptKind::AddFolder => tree.add(&format!("{}/", text.trim().trim_end_matches('/'))),
            PromptKind::Rename => tree.rename(&text),
            PromptKind::Copy => tree.copy_to(&text),
            PromptKind::Move => tree.move_to(&text),
            PromptKind::ConfirmDelete => {
                tree.delete().map(|()| selected.clone().unwrap_or_default())
            }
            PromptKind::ConfirmDiscard | PromptKind::Commit => return,
        };
        match result {
            Ok(path) => {
                self.message.clear();
                self.refresh_git();
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
        if std::mem::take(&mut self.tree_g) {
            return self.tree_git_key(key);
        }
        let Some(tree) = &mut self.tree else {
            self.focus = Focus::Editor;
            return;
        };
        match key {
            Key::Char('g') => self.tree_g = true,
            Key::Char('G') => tree.move_selection(isize::MAX),
            Key::Char('j') | Key::Down => tree.move_selection(1),
            Key::Char('k') | Key::Up => tree.move_selection(-1),
            Key::Char('a') => self.start_prompt(PromptKind::Add),
            Key::Char('A') => self.start_prompt(PromptKind::AddFolder),
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
            Key::Char('R') => {
                tree.refresh();
                self.refresh_git();
            }
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

    /// A key while the outline has the keyboard: `j` `k` move, Enter goes to the heading and back
    /// to the text, Tab or `o` goes without leaving, `q` hides the outline, Esc leaves it.
    fn outline_key(&mut self, key: Key) -> Vec<Effect> {
        let last = self.editor.headings().len().saturating_sub(1);
        match key {
            Key::Char('j') | Key::Down => self.outline_sel = (self.outline_sel + 1).min(last),
            Key::Char('k') | Key::Up => self.outline_sel = self.outline_sel.saturating_sub(1),
            Key::Enter | Key::Char('\t' | 'o') => {
                let offset = self
                    .editor
                    .headings()
                    .get(self.outline_sel)
                    .map(|h| h.offset);
                if let Some(offset) = offset {
                    self.editor.goto(offset);
                    if key == Key::Enter {
                        self.focus = Focus::Editor;
                    }
                    return vec![Effect::Align(ViewMove::Top)];
                }
            }
            Key::Char('q') => {
                self.toggle_setting(&Click::ToggleOutline);
                self.focus = Focus::Editor;
            }
            Key::Esc => self.focus = Focus::Editor,
            _ => {}
        }
        Vec::new()
    }

    /// Looks at git again: which repository holds the folder in the tree (or else the open file),
    /// what it says has changed, and the last commit's text of the open file for the margin.
    pub fn refresh_git(&mut self) {
        let folder = match (&self.tree, self.editor.path()) {
            (Some(tree), _) => Some(tree.root().to_path_buf()),
            (None, Some(path)) => path.parent().map(Path::to_path_buf),
            _ => None,
        }
        .map(|dir| {
            if dir.as_os_str().is_empty() {
                PathBuf::from(".")
            } else {
                dir
            }
        })
        .and_then(|dir| std::fs::canonicalize(dir).ok());
        self.git = folder.and_then(|folder| {
            let repo = match &self.git {
                Some(git) if git.folder == folder => git.repo.clone(),
                _ => Repo::discover(&folder)?,
            };
            let status = repo.status()?;
            Some(GitState {
                repo,
                status,
                folder,
            })
        });
        self.saves_seen = self.editor.saves;
        // Compare the open file with the last commit: a file that isn't in it yet is all new.
        let base = self.git.as_ref().and_then(|git| {
            let file = std::fs::canonicalize(self.editor.path()?).ok()?;
            match git.repo.head_text(&file) {
                Some(text) => Some(text),
                None => git.status.files.contains_key(&file).then(String::new),
            }
        });
        self.editor.set_base(base);
    }

    /// Looks at git again if the file has been saved since it last was looked at.
    fn look_after_save(&mut self) {
        if self.editor.saves != self.saves_seen && self.screen == Screen::Editing {
            self.refresh_git();
        }
    }

    /// Call when the window gets the focus back, since git may have changed in the meantime.
    pub fn focus_gained(&mut self) {
        if self.screen == Screen::Editing {
            self.refresh_git();
        }
    }

    /// The full path in the repository of the row selected in the tree.
    fn selected_in_repo(&self) -> Option<PathBuf> {
        let (git, tree) = (self.git.as_ref()?, self.tree.as_ref()?);
        let row = tree.selected()?;
        Some(git.folder.join(row.path.strip_prefix(tree.root()).ok()?))
    }

    /// The key after `g` in the tree: `a` stages, `u` unstages, `A` stages everything, `r`
    /// discards changes, `c` commits, `p` pushes, `l` pulls, and `g` goes to the top.
    fn tree_git_key(&mut self, key: Key) {
        if key == Key::Char('g') {
            if let Some(tree) = &mut self.tree {
                tree.move_selection(isize::MIN);
            }
            return;
        }
        let Some(git) = &self.git else {
            self.message = "Not a git repository".into();
            return;
        };
        let (repo, path) = (git.repo.clone(), self.selected_in_repo());
        let done = match (key, path) {
            (Key::Char('a'), Some(path)) => Some(repo.stage(&[path])),
            (Key::Char('u'), Some(path)) => Some(repo.unstage(&[path])),
            (Key::Char('A'), _) => Some(repo.stage_all()),
            (Key::Char('r'), Some(path)) => {
                match git.status.tone(&path) {
                    Some(_) => {
                        let name = path
                            .file_name()
                            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                        self.prompt = Some(Prompt {
                            label: format!("Discard changes to \u{201c}{name}\u{201d}?"),
                            text: path.to_string_lossy().into_owned(),
                            kind: PromptKind::ConfirmDiscard,
                        });
                    }
                    None => self.message = "No changes to discard".into(),
                }
                None
            }
            (Key::Char('c'), _) => {
                if git.status.files.is_empty() {
                    self.message = "Nothing to commit".into();
                } else {
                    self.prompt = Some(Prompt {
                        label: "Commit message: ".into(),
                        text: String::new(),
                        kind: PromptKind::Commit,
                    });
                }
                None
            }
            (Key::Char('p'), _) => {
                self.start_job("Push", &["push"]);
                None
            }
            (Key::Char('l'), _) => {
                self.start_job("Pull", &["pull", "--ff-only"]);
                None
            }
            _ => None,
        };
        if let Some(result) = done {
            self.message = result.err().unwrap_or_default();
            self.refresh_git();
        }
    }

    /// Commits what is staged, or everything if nothing is.
    fn git_commit(&mut self, message: &str) {
        let Some(git) = &self.git else { return };
        if git.status.staged() == 0
            && let Err(error) = git.repo.stage_all()
        {
            self.message = error;
            return;
        }
        self.message = match git.repo.commit(message) {
            Ok(summary) => summary,
            Err(error) => error,
        };
        self.refresh_git();
    }

    /// Throws away the changes under `path`.
    fn git_discard(&mut self, path: &Path) {
        let Some(git) = &self.git else { return };
        self.message = match git.repo.discard(&git.status, path) {
            Ok(()) => "Changes discarded".into(),
            Err(error) => error,
        };
        self.refresh_git();
        // The file on disk is what it was, so the open one follows unless it has edits.
        if !self.editor.buf.is_modified() && self.editor.path().is_some() {
            self.editor.reload();
        }
    }

    /// Runs `git` with `args` in the background, which the window keeps polling with `tick`.
    fn start_job(&mut self, label: &'static str, args: &[&str]) {
        if self.job.is_some() {
            self.message = "Git is still busy".into();
            return;
        }
        if let Some(git) = &self.git {
            self.job = Some(Job {
                label,
                done: git.repo.spawn(args),
            });
            self.message = format!("{label}\u{2026}");
        }
    }

    fn start_action(&mut self, action: StartAction) -> Vec<Effect> {
        match action {
            StartAction::OpenFile => vec![Effect::PickFile],
            StartAction::OpenFolder => vec![Effect::PickFolder],
            StartAction::Open(path) => {
                if path.is_dir() {
                    self.open_folder(&path);
                } else {
                    self.open_file(&path);
                }
                Vec::new()
            }
        }
    }

    /// A key press.
    pub fn key(&mut self, size: (f32, f32), key: Key) -> Vec<Effect> {
        let effects = self.key_pressed(size, key);
        self.look_after_save();
        effects
    }

    fn key_pressed(&mut self, size: (f32, f32), key: Key) -> Vec<Effect> {
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
        // The list of keys closes on any key; F1 opens it from anywhere.
        if std::mem::take(&mut self.help) {
            return Vec::new();
        }
        if key == Key::Help {
            self.help = true;
            return Vec::new();
        }
        // Esc closes an open menu instead of reaching the editor.
        if key == Key::Esc && self.popup.take().is_some() {
            return Vec::new();
        }
        // Ctrl-L goes to the file tree and Ctrl-H to the outline, showing them if hidden; the
        // same key again goes back to the text.
        if key == Key::Ctrl('l') {
            if self.focus != Focus::Tree {
                self.ensure_tree();
                self.focus = Focus::Tree;
            } else {
                self.focus = Focus::Editor;
            }
            return Vec::new();
        }
        if key == Key::Ctrl('h') {
            if self.focus != Focus::Outline {
                if !self.config.outline {
                    self.toggle_setting(&Click::ToggleOutline);
                }
                let cursor = self.editor.buf.cursor();
                self.outline_sel = outline::current(self.editor.headings(), cursor).unwrap_or(0);
                self.focus = Focus::Outline;
            } else {
                self.focus = Focus::Editor;
            }
            return Vec::new();
        }
        if self.focus == Focus::Tree {
            self.tree_key(key);
            return Vec::new();
        }
        if self.focus == Focus::Outline {
            return self.outline_key(key);
        }
        if key == Key::Ctrl('o') {
            if self.editor.buf.is_modified() {
                self.editor.vim.message = "E37: No write since last change".into();
                return Vec::new();
            }
            return vec![Effect::PickFile];
        }
        // The clipboard shortcuts: copy and cut act on the visual selection, paste wants the text.
        match key {
            Key::Ctrl('c') => {
                return self
                    .editor
                    .copy_selection()
                    .map(Effect::SetClipboard)
                    .into_iter()
                    .collect();
            }
            Key::Ctrl('x') => {
                return self
                    .editor
                    .cut_selection()
                    .map(Effect::SetClipboard)
                    .into_iter()
                    .collect();
            }
            Key::Ctrl('v') => return vec![Effect::ReadClipboard(ClipboardUse::Paste)],
            _ => {}
        }
        let mut effects = Vec::new();
        if self.editor.key(key) {
            effects.push(Effect::Quit);
        }
        self.help |= std::mem::take(&mut self.editor.help);
        if let Some(by) = self.editor.scroll.take() {
            let rows = (self.regions(size).editor.h / self.reading.row()) as usize;
            let (down, n) = match by {
                ViewMove::HalfDown => (true, rows / 2),
                ViewMove::HalfUp => (false, rows / 2),
                ViewMove::PageDown => (true, rows.saturating_sub(2)),
                ViewMove::PageUp => (false, rows.saturating_sub(2)),
                align => {
                    effects.push(Effect::Align(align));
                    (true, 0)
                }
            };
            if n > 0 {
                // The cursor keeps its place on the screen as the text moves past it.
                self.editor.move_rows(down, n);
                effects.push(Effect::ScrollRows(if down {
                    n as i32
                } else {
                    -(n as i32)
                }));
            }
        }
        // Yanks go out to the clipboard, and `"+` and `"*` come in from it.
        effects.extend(
            self.editor
                .vim
                .take_clipboard_write()
                .map(Effect::SetClipboard),
        );
        if self.editor.vim.wants_clipboard() {
            effects.push(Effect::ReadClipboard(ClipboardUse::Register));
        }
        effects
    }

    /// Hands over what the system clipboard held, as asked for by `Effect::ReadClipboard`.
    pub fn clipboard_text(&mut self, purpose: ClipboardUse, text: String) {
        match purpose {
            ClipboardUse::Register => self.editor.vim.set_clipboard_register(text),
            ClipboardUse::Paste => self.editor.paste_text(&text),
        }
    }

    /// Call when the open file may have changed on disk. With no unsaved changes it is read again;
    /// with some, they are kept and the status bar says how to choose. True if anything showed.
    pub fn file_changed_on_disk(&mut self) -> bool {
        if self.screen != Screen::Editing || !self.editor.changed_on_disk() {
            return false;
        }
        let name = self.editor.file_name();
        if self.editor.buf.is_modified() {
            self.message = format!("{name} changed on disk; :e! reloads it, :w overwrites it");
        } else {
            self.editor.reload();
            self.message = format!("Reloaded {name}, which changed on disk");
        }
        true
    }

    /// The name to offer for an export: the document's own name with the new extension, in its
    /// folder, or `untitled` in the current one.
    pub fn export_suggestion(&self, kind: ExportKind) -> PathBuf {
        let extension = match kind {
            ExportKind::Markdown => "md",
            ExportKind::Pdf => "pdf",
        };
        match self.editor.path() {
            Some(path) => path.with_extension(extension),
            None => PathBuf::from(format!("untitled.{extension}")),
        }
    }

    /// Writes the document to `path` as Markdown or PDF, and says on the status bar how it went.
    pub fn export_to(&mut self, kind: ExportKind, path: &Path) {
        let text = self.editor.buf.text();
        let bytes = match kind {
            ExportKind::Markdown => crate::export::to_markdown(text).into_bytes(),
            ExportKind::Pdf => {
                let title = path
                    .file_stem()
                    .map_or_else(String::new, |s| s.to_string_lossy().into_owned());
                crate::pdf::to_pdf(text, &title)
            }
        };
        self.message = match std::fs::write(path, bytes) {
            Ok(()) => format!("Exported {}", path.display()),
            Err(error) => format!("Can't write {}: {error}", path.display()),
        };
    }

    /// Opens `path` for editing and puts it first in the recent files. If it can't be read the
    /// start screen says why.
    pub fn open_file(&mut self, path: &Path) {
        match Editor::open(path) {
            Ok(editor) => {
                self.editor = editor;
                self.opened += 1;
                self.screen = Screen::Editing;
                self.popup = None;
                self.hover = None;
                self.remember(path);
                self.refresh_git();
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

    /// Puts a file or folder first in the recent list and saves the list.
    fn remember(&mut self, path: &Path) {
        let absolute = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        self.recent.add(absolute);
        if let Some(saved) = &self.recent_path {
            // A recent list that won't save isn't worth interrupting writing for.
            let _ = self.recent.save(saved);
        }
    }

    /// Shows `folder`'s files in the tree on the right, ready to edit in a new document, and puts
    /// it first in the recent list.
    pub fn open_folder(&mut self, folder: &Path) {
        self.remember(folder);
        self.tree = Some(Tree::new(folder));
        self.refresh_git();
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
        // The folder is remembered with the files, and opens again from the start screen.
        let canonical = std::fs::canonicalize(&dir).unwrap();
        assert_eq!(app.recent.files(), [canonical]);
        app.screen = Screen::Start {
            message: String::new(),
        };
        app.tree = None;
        let items = start::layout(SIZE, app.recent.files());
        assert!(items[2].title.ends_with("bwa_app_folder/"));
        app.click(
            SIZE,
            (items[2].bounds.x + 5.0, items[2].bounds.y + 5.0),
            |_, _| None,
        );
        assert_eq!(app.screen, Screen::Editing);
        assert_eq!(app.tree.as_ref().unwrap().rows().len(), 1);
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
        // The font button opens a list, whose choice sets how the text is read and leaves the text
        // itself alone. It is remembered in the settings.
        assert_eq!(app.reading.family, "Serif");
        app.click(SIZE, button("Font"), |_, _| None);
        let popup = app.popup.clone().unwrap();
        assert!(popup.items.iter().any(|i| i.selected && i.label == "Serif"));
        let georgia = popup.items.iter().find(|i| i.label == "Georgia").unwrap();
        app.click(
            SIZE,
            (georgia.bounds.x + 3.0, georgia.bounds.y + 3.0),
            |_, _| None,
        );
        assert!(app.popup.is_none());
        assert_eq!(app.reading.family, "Georgia");
        app.click(SIZE, button("Size"), |_, _| None);
        let popup = app.popup.clone().unwrap();
        let big = popup.items.iter().find(|i| i.label == "24").unwrap();
        app.click(SIZE, (big.bounds.x + 3.0, big.bounds.y + 3.0), |_, _| None);
        assert_eq!(app.reading.size_pt, 24.0);
        assert_eq!(app.editor.buf.text(), "one *two* three");
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

    /// Types `keys`: `\n` is Enter, `\x1b` Esc, `\x08` Backspace, `\x05` Ctrl+L, and `\t` Tab.
    fn press(app: &mut App, keys: &str) {
        for c in keys.chars() {
            app.key(
                SIZE,
                match c {
                    '\n' => Key::Enter,
                    '\x1b' => Key::Esc,
                    '\x08' => Key::Backspace,
                    '\x05' => Key::Ctrl('l'),
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
        // Add: a dialog asks for the name, and shows what is typed; a file is made and opened.
        press(&mut app, "a");
        press(&mut app, "ne");
        let dialog = app.chrome(SIZE).over_labels;
        assert!(dialog.iter().any(|l| l.text == "New file:"));
        assert!(dialog.iter().any(|l| l.text.starts_with("ne\u{258f}")));
        press(&mut app, "w.dj\n");
        assert!(root.join("new.dj").is_file());
        assert_eq!(app.editor.file_name(), "new.dj");
        assert!(app.prompt.is_none());
        // A name that ends in / is a folder, A makes one whatever is typed, and Esc drops a prompt.
        press(&mut app, "a");
        press(&mut app, "more/\n");
        assert!(root.join("more").is_dir());
        press(&mut app, "Aplain\n");
        assert!(root.join("more/plain").is_dir());
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
        // Delete asks first, in a dialog with buttons: n keeps it, y removes it, and the document
        // lets go of the file.
        press(&mut app, "d");
        let ask = app.chrome(SIZE).over_labels;
        assert!(
            ask.iter()
                .any(|l| l.text.starts_with("Delete \u{201c}moved.dj"))
        );
        assert!(ask.iter().any(|l| l.text == "Delete (y)"));
        press(&mut app, "n");
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
                panel.y + ui::TREE_TOP + i * ui::TREE_ROW + 4.0,
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
        let capacity = ui::tree_capacity(app.regions(SIZE).tree.unwrap(), ui::TREE_TOP);
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

    #[test]
    fn the_command_and_search_lines_show_on_the_status_bar_as_they_are_typed() {
        let mut app = App::new(Editor::default(), Screen::Editing, Recent::default(), None);
        press(&mut app, "ithe dark night\x1b0");
        let status = |app: &mut App| -> String {
            let bar = app.regions(SIZE).status;
            let chrome = app.chrome(SIZE);
            let label = chrome
                .labels
                .iter()
                .find(|l| l.y >= bar.y && l.x > bar.x + 80.0 && l.x < bar.x + 100.0);
            label.map(|l| l.text.clone()).unwrap_or_default()
        };
        assert_eq!(status(&mut app), "[No Name] [+]");
        press(&mut app, ":");
        assert_eq!(status(&mut app), ":\u{258f}");
        press(&mut app, "wq");
        assert_eq!(status(&mut app), ":wq\u{258f}");
        press(&mut app, "\x08\x08\x1b");
        assert_eq!(status(&mut app), "[No Name] [+]");
        // A search shows its pattern, then what it found or didn't.
        press(&mut app, "/dark");
        assert_eq!(status(&mut app), "/dark\u{258f}");
        press(&mut app, "\n");
        assert_eq!(app.editor.buf.cursor(), 4);
        assert_eq!(status(&mut app), "[No Name] [+]");
        press(&mut app, "/zzz\n");
        assert_eq!(status(&mut app), "E486: Pattern not found: zzz");
        // The next key clears the note.
        press(&mut app, "l");
        assert_eq!(status(&mut app), "[No Name] [+]");
    }

    #[test]
    fn the_save_and_auto_save_buttons() {
        let dir = std::env::temp_dir().join("bwa_app_autosave");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (file, settings) = (dir.join("book.dj"), dir.join("config.toml"));
        std::fs::write(&file, "one two").unwrap();
        let mut app = App::new(
            Editor::open(&file).unwrap(),
            Screen::Editing,
            Recent::default(),
            None,
        );
        app.set_config(Config::default(), Some(settings.clone()));
        let toolbar = toolbar::toolbar(app.regions(SIZE).toolbar);
        let button = |label: &str| {
            let b = toolbar
                .buttons
                .iter()
                .find(|b| b.label == label)
                .unwrap()
                .bounds;
            (b.x + b.w / 2.0, b.y + b.h / 2.0)
        };
        // Save does what :w does.
        press(&mut app, "x");
        assert!(app.editor.buf.is_modified());
        app.click(SIZE, button("Save"), |_, _| None);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "ne two");
        assert!(!app.editor.buf.is_modified());
        // Auto toggles, is lit while on, and is remembered in the settings file.
        assert!(!app.autosave.enabled);
        app.click(SIZE, button("Auto"), |_, _| None);
        assert!(app.autosave.enabled);
        assert!(Config::load(&settings).auto_save);
        let lit = |app: &mut App| {
            let (x, y) = button("Auto");
            app.chrome(SIZE).rects.iter().any(|r| {
                r.color == [0xd2, 0xc8, 0xac, 0xff]
                    && r.x <= x
                    && x < r.x + r.w
                    && r.y <= y
                    && y < r.y + r.h
            })
        };
        assert!(lit(&mut app));
        app.click(SIZE, button("Auto"), |_, _| None);
        assert!(!app.autosave.enabled && !lit(&mut app));
        assert!(!Config::load(&settings).auto_save);
        // The theme button switches the look and the choice is remembered.
        assert!(!app.theme().dark);
        app.click(SIZE, button("\u{263e}"), |_, _| None);
        assert!(app.theme().dark && Config::load(&settings).dark);
        app.click(SIZE, button("\u{263e}"), |_, _| None);
        assert!(!app.theme().dark && !Config::load(&settings).dark);
        // A tip shows once the mouse has rested on a button, and goes when it moves off.
        let (x, y) = button("Save");
        let shown = |app: &mut App| {
            app.chrome(SIZE)
                .over_labels
                .iter()
                .any(|l| l.text.starts_with("Save ("))
        };
        assert!(app.hover_at(SIZE, x, y) && !shown(&mut app));
        assert!(app.next_wake().is_some());
        app.tick(Instant::now() + Duration::from_secs(1));
        assert!(shown(&mut app));
        app.hover_at(SIZE, 500.0, 300.0);
        assert!(!shown(&mut app));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn git_marks_files_in_the_tree_and_stages_commits_and_discards_from_it() {
        use crate::diff::Change;
        use crate::git::Tone;
        use std::process::Command;
        let dir = std::env::temp_dir().join("bwa_app_git");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let git = |args: &[&str]| {
            Command::new("git")
                .arg("-C")
                .arg(&dir)
                .args(args)
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
        };
        if !git(&["init", "-q", "-b", "main"]) {
            return;
        }
        git(&["config", "user.name", "T"]);
        git(&["config", "user.email", "t@example.com"]);
        std::fs::write(dir.join("a.dj"), "one\ntwo\nthree\n").unwrap();
        std::fs::write(dir.join("b.dj"), "b\n").unwrap();
        git(&["add", "-A"]);
        git(&["commit", "-q", "-m", "start"]);
        // An edit, a new file, and a new file in a folder.
        std::fs::write(dir.join("a.dj"), "one\nTWO\nthree\n").unwrap();
        std::fs::write(dir.join("c.dj"), "c\n").unwrap();
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub/d.dj"), "d\n").unwrap();

        let mut app = start_app(Recent::default());
        app.open_folder(&dir);
        let tone = |app: &mut App, name: &str| {
            let panel = app.regions(SIZE).tree;
            let view = app.tree_view(panel).unwrap();
            view.rows
                .into_iter()
                .find(|r| r.name == name)
                .and_then(|r| r.git)
        };
        assert_eq!(tone(&mut app, "a.dj"), Some((Tone::Modified, Some('M'))));
        assert_eq!(tone(&mut app, "c.dj"), Some((Tone::Untracked, Some('U'))));
        assert_eq!(tone(&mut app, "sub"), Some((Tone::Untracked, None)));
        assert_eq!(tone(&mut app, "b.dj"), None);
        let panel = app.regions(SIZE).tree;
        let info = app.tree_view(panel).unwrap().git.unwrap();
        assert_eq!(
            (info.branch.as_str(), info.changes, info.staged),
            ("main", 3, 0)
        );
        assert!(
            app.chrome(SIZE)
                .labels
                .iter()
                .any(|l| l.text.starts_with("main*"))
        );

        // Opening the edited file marks the line that changed against the last commit.
        app.open_file(&dir.join("a.dj"));
        assert_eq!(app.editor.view().changes, [(1, Change::Modified)]);
        // Stage it with `ga` on the tree, then commit with `gc` and a message.
        press(&mut app, "\x05");
        while rows(&app)[app.tree.as_ref().unwrap().selected_index()] != "a.dj*" {
            press(&mut app, "j");
        }
        press(&mut app, "ga");
        assert!(
            app.git.as_ref().unwrap().status.files[&dir.canonicalize().unwrap().join("a.dj")]
                .staged()
        );
        press(&mut app, "gc");
        let ask = app.chrome(SIZE).over_labels;
        assert!(ask.iter().any(|l| l.text == "Commit message:"));
        assert!(
            ask.iter()
                .any(|l| l.text.starts_with("Enter to commit what is staged"))
        );
        press(&mut app, "edit two\n");
        assert!(app.message.contains("edit two"), "{}", app.message);
        assert!(tone(&mut app, "a.dj").is_none());
        // The margin now compares with the new commit.
        assert!(app.editor.view().changes.is_empty());
        // Discarding asks first and puts the committed text back.
        std::fs::write(dir.join("b.dj"), "changed\n").unwrap();
        press(&mut app, "R");
        while rows(&app)[app.tree.as_ref().unwrap().selected_index()] != "b.dj*" {
            press(&mut app, "j");
        }
        press(&mut app, "gr");
        assert!(
            app.chrome(SIZE)
                .over_labels
                .iter()
                .any(|l| l.text.starts_with("Discard changes to"))
        );
        press(&mut app, "n");
        assert_eq!(
            std::fs::read_to_string(dir.join("b.dj")).unwrap(),
            "changed\n"
        );
        press(&mut app, "gry");
        assert_eq!(std::fs::read_to_string(dir.join("b.dj")).unwrap(), "b\n");
        assert!(tone(&mut app, "b.dj").is_none());
        // A folder outside any repository has no git at all.
        let plain = std::env::temp_dir().join("bwa_app_no_git");
        let _ = std::fs::remove_dir_all(&plain);
        std::fs::create_dir_all(&plain).unwrap();
        app.open_folder(&plain);
        assert!(app.git.is_none());
        press(&mut app, "ga");
        assert_eq!(app.message, "Not a git repository");
        std::fs::remove_dir_all(dir).unwrap();
        std::fs::remove_dir_all(plain).unwrap();
    }

    #[test]
    fn the_outline_lists_headings_marks_the_current_one_and_jumps_on_click() {
        let path = temp_file("outline", "# One\n\ntext\n\n## Two\n\nmore\n\n## Three\n");
        let mut app = App::new(
            Editor::open(&path).unwrap(),
            Screen::Editing,
            Recent::default(),
            None,
        );
        let settings = std::env::temp_dir().join("bwa_outline_settings.toml");
        app.set_config(Config::default(), Some(settings.clone()));
        let panel = app.regions(SIZE).outline.unwrap();
        let rows = |app: &mut App| {
            let panel = app.regions(SIZE).outline;
            app.outline_view(panel)
                .unwrap()
                .rows
                .into_iter()
                .map(|r| (r.title, r.current))
                .collect::<Vec<_>>()
        };
        let row = |i: f32| {
            (
                panel.x + 30.0,
                panel.y + ui::TREE_HEADER + i * ui::TREE_ROW + 4.0,
            )
        };
        assert_eq!(
            rows(&mut app),
            [
                ("One".into(), true),
                ("Two".into(), false),
                ("Three".into(), false)
            ]
        );
        // Clicking a heading moves the cursor there and asks for it at the top of the view.
        assert_eq!(
            app.click(SIZE, row(2.0), |_, _| None),
            [Effect::Align(ViewMove::Top)]
        );
        assert_eq!(
            app.editor.buf.cursor(),
            "# One\n\ntext\n\n## Two\n\nmore\n\n## ".len()
        );
        assert_eq!(rows(&mut app)[2], ("Three".into(), true));
        // Ctrl-H gives the outline the keyboard, on the current heading; j and k move, and Enter
        // goes there and back to the text.
        assert_eq!(app.key(SIZE, Key::Ctrl('h')), []);
        assert_eq!(app.focus, Focus::Outline);
        app.key(SIZE, Key::Char('k'));
        let selected = |app: &mut App| {
            let panel = app.regions(SIZE).outline;
            let rows = app.outline_view(panel).unwrap().rows;
            rows.iter().position(|r| r.selected)
        };
        assert_eq!(selected(&mut app), Some(1));
        app.key(SIZE, Key::Char('j'));
        app.key(SIZE, Key::Char('k'));
        app.key(SIZE, Key::Char('k'));
        assert_eq!(app.key(SIZE, Key::Enter), [Effect::Align(ViewMove::Top)]);
        assert_eq!(app.focus, Focus::Editor);
        assert_eq!(app.editor.buf.cursor(), 2);
        // `q` in the outline hides it and the editor takes its room; the choice is remembered.
        let width = app.regions(SIZE).editor.w;
        app.key(SIZE, Key::Ctrl('h'));
        app.key(SIZE, Key::Char('q'));
        assert!(app.regions(SIZE).outline.is_none() && app.focus == Focus::Editor);
        assert_eq!(app.regions(SIZE).editor.w, width + ui::OUTLINE_WIDTH);
        assert!(!Config::load(&settings).outline);
        // Ctrl-H shows it again.
        app.key(SIZE, Key::Ctrl('h'));
        assert!(app.regions(SIZE).outline.is_some() && app.focus == Focus::Outline);
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_file(&settings).unwrap();
    }

    #[test]
    fn page_keys_move_the_cursor_and_ask_the_window_to_scroll_by_as_many_rows() {
        let text: String = (0..200).map(|i| format!("line {i}\n")).collect();
        let path = temp_file("pages", &text);
        let mut app = App::new(
            Editor::open(&path).unwrap(),
            Screen::Editing,
            Recent::default(),
            None,
        );
        let rows = (app.regions(SIZE).editor.h / app.reading.row()) as usize;
        let line = |app: &App| {
            app.editor.buf.text()[..app.editor.buf.cursor()]
                .matches('\n')
                .count()
        };
        let half = (rows / 2) as i32;
        assert_eq!(app.key(SIZE, Key::Ctrl('d')), [Effect::ScrollRows(half)]);
        assert_eq!(line(&app), half as usize);
        assert_eq!(app.key(SIZE, Key::Ctrl('u')), [Effect::ScrollRows(-half)]);
        assert_eq!(line(&app), 0);
        let page = rows as i32 - 2;
        assert_eq!(app.key(SIZE, Key::Ctrl('f')), [Effect::ScrollRows(page)]);
        assert_eq!(line(&app), page as usize);
        assert_eq!(app.key(SIZE, Key::Char('z')), []);
        assert_eq!(
            app.key(SIZE, Key::Char('z')),
            [Effect::Align(ViewMove::Center)]
        );
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn auto_save_writes_the_file_once_the_text_has_been_still() {
        let path = temp_file("autosave", "hello");
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let mut app = App::new(
            Editor::open(&path).unwrap(),
            Screen::Editing,
            Recent::default(),
            None,
        );
        app.set_config(
            Config {
                auto_save: true,
                ..Config::default()
            },
            None,
        );
        // Nothing changed: nothing to wake for.
        app.tick(at(10_000));
        assert_eq!(app.next_wake(), None);
        press(&mut app, "x");
        app.tick(at(10_100));
        assert_eq!(app.next_wake(), Some(at(12_100)));
        app.tick(at(11_000));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello");
        // More typing pushes the save back.
        press(&mut app, "x");
        app.tick(at(11_500));
        app.tick(at(13_000));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello");
        app.tick(at(13_500));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "llo");
        assert!(!app.editor.buf.is_modified());
        // Off, it leaves changes for the user to save.
        app.autosave.enabled = false;
        press(&mut app, "x");
        app.tick(at(20_000));
        app.tick(at(30_000));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "llo");
        std::fs::remove_file(path).unwrap();
        // A document with no file has nowhere to save, and doesn't complain.
        let mut app = App::new(Editor::default(), Screen::Editing, Recent::default(), None);
        app.set_config(
            Config {
                auto_save: true,
                ..Config::default()
            },
            None,
        );
        press(&mut app, "ix");
        app.tick(at(100_000));
        app.tick(at(200_000));
        assert!(app.editor.buf.is_modified());
        assert!(app.editor.vim.message.is_empty());
    }

    #[test]
    fn export_buttons_ask_for_a_place_and_export_writes_the_file() {
        let path = temp_file(
            "export",
            "# Title\n\n*bold* and [red]{style=\"color:#c00\"} text\n",
        );
        let mut app = App::new(
            Editor::open(&path).unwrap(),
            Screen::Editing,
            Recent::default(),
            None,
        );
        let toolbar = toolbar::toolbar(app.regions(SIZE).toolbar);
        let button = |label: &str| {
            let b = toolbar
                .buttons
                .iter()
                .find(|b| b.label == label)
                .unwrap()
                .bounds;
            (b.x + b.w / 2.0, b.y + b.h / 2.0)
        };
        assert_eq!(
            app.click(SIZE, button("MD"), |_, _| None),
            [Effect::PickExport(ExportKind::Markdown)]
        );
        assert_eq!(
            app.click(SIZE, button("PDF"), |_, _| None),
            [Effect::PickExport(ExportKind::Pdf)]
        );
        // The suggested name is the file's own with the new extension.
        let markdown = app.export_suggestion(ExportKind::Markdown);
        assert_eq!(markdown, path.with_extension("md"));
        assert_eq!(
            app.export_suggestion(ExportKind::Pdf),
            path.with_extension("pdf")
        );
        assert_eq!(
            App::new(Editor::default(), Screen::Editing, Recent::default(), None)
                .export_suggestion(ExportKind::Pdf),
            PathBuf::from("untitled.pdf")
        );
        // Markdown is lossy about style but keeps the text; PDF is a PDF.
        app.export_to(ExportKind::Markdown, &markdown);
        assert_eq!(
            std::fs::read_to_string(&markdown).unwrap(),
            "# Title\n\n**bold** and red text\n"
        );
        assert!(app.message.starts_with("Exported "));
        let pdf = path.with_extension("pdf");
        app.export_to(ExportKind::Pdf, &pdf);
        assert!(std::fs::read(&pdf).unwrap().starts_with(b"%PDF-"));
        // A place that can't be written to is reported.
        app.export_to(ExportKind::Markdown, Path::new("/nonexistent/dir/out.md"));
        assert!(
            app.message
                .starts_with("Can't write /nonexistent/dir/out.md"),
            "{}",
            app.message
        );
        // The note shows on the status bar until the next key.
        assert!(
            app.chrome(SIZE)
                .labels
                .iter()
                .any(|l| l.text.starts_with("Can't write"))
        );
        press(&mut app, "l");
        assert!(
            !app.chrome(SIZE)
                .labels
                .iter()
                .any(|l| l.text.starts_with("Can't write"))
        );
        for file in [path, markdown, pdf] {
            std::fs::remove_file(file).unwrap();
        }
    }

    #[test]
    fn clipboard_shortcuts_and_registers_ask_the_window_to_move_text() {
        let mut app = App::new(Editor::default(), Screen::Editing, Recent::default(), None);
        press(&mut app, "ione two three\x1b0");
        let key = |app: &mut App, key: Key| app.key(SIZE, key);
        // Copy and cut work on the visual selection; with none, they do nothing.
        assert_eq!(key(&mut app, Key::Ctrl('c')), []);
        press(&mut app, "ve");
        assert_eq!(
            key(&mut app, Key::Ctrl('c')),
            [Effect::SetClipboard("one".into())]
        );
        assert_eq!(app.editor.buf.text(), "one two three");
        assert_eq!(
            key(&mut app, Key::Ctrl('x')),
            [Effect::SetClipboard("one".into())]
        );
        assert_eq!(app.editor.buf.text(), " two three");
        // Paste asks for the text and then puts it in.
        assert_eq!(
            key(&mut app, Key::Ctrl('v')),
            [Effect::ReadClipboard(ClipboardUse::Paste)]
        );
        app.clipboard_text(ClipboardUse::Paste, "ONE".into());
        assert_eq!(app.editor.buf.text(), " ONEtwo three");
        // A yank goes out on its own; "+ asks for the text before its command runs.
        let effects = app.key(SIZE, Key::Char('y'));
        assert!(effects.is_empty());
        assert_eq!(
            app.key(SIZE, Key::Char('y')),
            [Effect::SetClipboard(" ONEtwo three\n".into())]
        );
        assert!(app.key(SIZE, Key::Char('"')).is_empty());
        assert_eq!(
            app.key(SIZE, Key::Char('+')),
            [Effect::ReadClipboard(ClipboardUse::Register)]
        );
        app.clipboard_text(ClipboardUse::Register, "outside".into());
        app.key(SIZE, Key::Char('p'));
        assert!(
            app.editor.buf.text().contains("outside"),
            "{}",
            app.editor.buf.text()
        );
    }

    #[test]
    fn a_change_on_disk_reloads_or_warns() {
        let path = temp_file("changed", "first");
        let mut app = App::new(
            Editor::open(&path).unwrap(),
            Screen::Editing,
            Recent::default(),
            None,
        );
        // Nothing changed, nothing to do; and our own save isn't a change.
        assert!(!app.file_changed_on_disk());
        press(&mut app, "x:w\n");
        assert!(!app.file_changed_on_disk());
        // Unmodified, it reloads and says so.
        std::fs::write(&path, "second").unwrap();
        assert!(app.file_changed_on_disk());
        assert_eq!(app.editor.buf.text(), "second");
        assert!(app.message.starts_with("Reloaded "), "{}", app.message);
        assert!(!app.file_changed_on_disk());
        // Modified, it keeps your work and says how to choose.
        press(&mut app, "x");
        std::fs::write(&path, "third").unwrap();
        assert!(app.file_changed_on_disk());
        assert_eq!(app.editor.buf.text(), "econd");
        assert!(app.message.contains(":e! reloads it"), "{}", app.message);
        press(&mut app, ":e!\n");
        assert_eq!(app.editor.buf.text(), "third");
        std::fs::remove_file(path).unwrap();
    }
}
