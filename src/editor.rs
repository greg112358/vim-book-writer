use crate::action::Action;
use crate::blocks::{set_heading, toggle_list, toggle_quote};
use crate::buffer::Buffer;
use crate::display::Display;
use crate::document::Document;
use crate::format::{
    Mark, set_color, set_font_family, set_font_size, toggle, toggle_link, toggle_underline,
    uppercase,
};
use crate::motion::{word_back, word_end};
use crate::outline::{self, Heading};
use crate::ui::Spot;
use crate::vim::{Command, Key, Mode, ViewMove, Vim};
use std::io;
use std::ops::Range;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_VERSION: AtomicU64 = AtomicU64::new(1);

/// What the window needs to draw a frame.
pub struct View<'a> {
    pub display: &'a Display,
    /// Changes whenever `display` does.
    pub version: u64,
    pub mode: Mode,
    /// The cursor, as an offset in `display.text`.
    pub cursor: usize,
    /// The visual selection, as offsets in `display.text`.
    pub selection: Option<Range<usize>>,
    /// Where the searched-for text is, as offsets in `display.text`, in order.
    pub found: &'a [Range<usize>],
}

/// What the window shows while a `:s` is being typed: the text with it done.
#[derive(Debug)]
struct Preview {
    display: Display,
    version: u64,
    cursor: usize,
    found: Vec<Range<usize>>,
}

/// The document being edited: its text and cursor, the vim state, and the file behind it.
#[derive(Debug, Default)]
pub struct Editor {
    pub buf: Buffer,
    pub vim: Vim,
    doc: Document,
    /// What the screen shows, and the buffer revision and mode it was made for.
    display: Display,
    display_key: Option<(u64, bool)>,
    /// Differs from every earlier version, of any editor, whenever `display` is rebuilt, so a
    /// renderer can tell when it changed.
    display_version: u64,
    /// The word count and the buffer revision it was counted at.
    words: Option<(u64, usize)>,
    /// `spot` for the buffer revision, cursor and selection it was worked out at.
    spot: Option<((u64, usize, usize, usize), Spot)>,
    /// The `display_key` that `buf.rows` were laid out for.
    rows_key: Option<(u64, bool)>,
    /// A scroll the last key asked for, for the app to carry out.
    pub scroll: Option<ViewMove>,
    /// `:help` was typed, for the app to show the list of keys.
    pub help: bool,
    /// Where the highlighted search text is in `display`, and the display version and pattern
    /// that was worked out for.
    found: Vec<Range<usize>>,
    found_for: (u64, String),
    /// The text a `:s` being typed would make, and the revision and command it was made for.
    preview: Option<Preview>,
    preview_for: (u64, String),
    /// The headings, and the buffer revision they were read at.
    headings: Option<(u64, Vec<Heading>)>,
}

impl Editor {
    pub fn open(path: &Path) -> io::Result<Self> {
        let doc = Document::open(path)?;
        let mut editor = Self {
            buf: Buffer::new(doc.text()),
            vim: Vim::new(),
            doc,
            display: Display::default(),
            display_key: None,
            display_version: 0,
            words: None,
            spot: None,
            rows_key: None,
            scroll: None,
            help: false,
            found: Vec::new(),
            found_for: (0, String::new()),
            preview: None,
            preview_for: (0, String::new()),
            headings: None,
        };
        // A file that starts with hidden syntax, like `# `, opens with the cursor on the text.
        editor.snap_cursor(0, Key::Esc);
        Ok(editor)
    }

    /// The number of words, counted again only when the text has changed.
    pub fn word_count(&mut self) -> usize {
        let revision = self.buf.revision();
        match self.words {
            Some((at, words)) if at == revision => words,
            _ => {
                let words = self.buf.text().split_whitespace().count();
                self.words = Some((revision, words));
                words
            }
        }
    }

    /// The word the cursor is on, or how many words are selected. Worked out again only when the
    /// text, cursor or selection moves.
    pub fn spot(&mut self) -> Spot {
        let (cursor, selection) = (self.buf.cursor(), self.vim.selection(&self.buf));
        let (from, to) = selection
            .as_ref()
            .map_or((0, usize::MAX), |r| (r.start, r.end));
        let key = (self.buf.revision(), cursor, from, to);
        if let Some((at, spot)) = &self.spot
            && *at == key
        {
            return *spot;
        }
        let text = self.buf.text();
        let spot = match &selection {
            Some(range) => Spot::Selected(text[range.clone()].split_whitespace().count()),
            None => {
                // Up to and including the char under the cursor, so a cursor on a word's first
                // letter is on that word and one in the blanks after it is still on it.
                let end = cursor + text[cursor..].chars().next().map_or(0, char::len_utf8);
                Spot::Word(text[..end].split_whitespace().count())
            }
        };
        self.spot = Some((key, spot));
        spot
    }

    /// The headings of the text, read again only when it has changed.
    pub fn headings(&mut self) -> &[Heading] {
        let revision = self.buf.revision();
        match &mut self.headings {
            Some((at, _)) if *at == revision => {}
            slot => *slot = Some((revision, outline::headings(self.buf.text()))),
        }
        &self.headings.as_ref().unwrap().1
    }

    /// The font family of the selection, or of the text under the cursor: `None` if the selection
    /// holds more than one. Text with no family of its own is in the body font, `Serif`.
    pub fn current_font(&mut self) -> Option<String> {
        let cursor = self.buf.cursor();
        let selection = self.vim.selection(&self.buf);
        let (display, _) = self.display();
        let families = |range: Range<usize>| -> Vec<Option<&str>> {
            let mut found: Vec<Option<&str>> = display
                .spans
                .iter()
                .filter(|s| !s.faint && s.source.start < range.end && s.source.end > range.start)
                .map(|s| s.format.style.family.as_deref())
                .collect();
            found.dedup();
            found
        };
        // With no selection, the char under the cursor, or the one before it at the end.
        let mut found = families(selection.unwrap_or(cursor..cursor + 1));
        if found.is_empty() {
            found = families(cursor.saturating_sub(1)..cursor);
        }
        match found.as_slice() {
            [family] => Some(family.unwrap_or("Serif").to_string()),
            _ => None,
        }
    }

    /// Puts the cursor at source offset `offset`, moved off any hidden syntax there.
    pub fn goto(&mut self, offset: usize) {
        self.buf.set_cursor(offset);
        self.snap_cursor(0, Key::Esc);
    }

    /// The file being edited, if it has one.
    pub fn path(&self) -> Option<&Path> {
        self.doc.path()
    }

    /// Says the text now belongs to another file, or none, because the file was moved or deleted.
    pub fn set_path(&mut self, path: Option<std::path::PathBuf>) {
        self.doc.set_path(path);
    }

    /// The name of the file, without its folder, or `[No Name]`.
    pub fn file_name(&self) -> String {
        self.doc
            .path()
            .and_then(|path| path.file_name())
            .map_or_else(
                || "[No Name]".to_string(),
                |name| name.to_string_lossy().into_owned(),
            )
    }

    /// What to draw: the source with faint syntax while inserting, and the finished-looking text
    /// otherwise. It is rebuilt only when the text or that choice has changed since the last call,
    /// and the number that goes with it changes whenever it is.
    pub fn display(&mut self) -> (&Display, u64) {
        let key = (self.buf.revision(), self.vim.mode == Mode::Insert);
        if self.display_key != Some(key) {
            self.display = if key.1 {
                Display::insert(self.buf.text())
            } else {
                Display::normal(self.buf.text())
            };
            self.display_key = Some(key);
            // Numbered across all editors, so the window never mistakes a newly opened file's
            // display for the old one's.
            self.display_version = NEXT_VERSION.fetch_add(1, Ordering::Relaxed);
        }
        (&self.display, self.display_version)
    }

    /// Tells the editor where the window wrapped the text it last drew, as displayed offsets of
    /// where each row starts, so up and down move a row at a time.
    pub fn set_rows(&mut self, rows: &[usize]) {
        if self.rows_key == self.display_key {
            return;
        }
        self.buf.rows.clear();
        self.buf
            .rows
            .extend(rows.iter().map(|&at| self.display.to_source(at)));
        self.rows_key = self.display_key;
    }

    /// Moves the cursor down or up `rows` rows, or as far as it can go: wrapped rows when the
    /// window has laid the text out, else whole lines.
    pub fn move_rows(&mut self, down: bool, rows: usize) {
        let before = self.buf.cursor();
        let inserting = self.vim.mode == Mode::Insert;
        for _ in 0..rows {
            let moved = if !self.buf.rows.is_empty() {
                self.buf.move_row(down, inserting)
            } else if down && self.buf.line_end() + 1 < self.buf.text().len() {
                self.buf.move_down();
                true
            } else if !down && self.buf.line_start() > 0 {
                self.buf.move_up();
                true
            } else {
                false
            };
            if !moved {
                break;
            }
        }
        if !inserting {
            self.buf.clamp_to_char();
            self.snap_cursor(before, Key::Char(if down { 'j' } else { 'k' }));
        }
    }

    /// Feeds a key to the vim engine and carries out what it asks for. True means the app
    /// should quit.
    pub fn key(&mut self, key: Key) -> bool {
        // Rows laid out for other text, or the other mode's display, say nothing about this one.
        if self.rows_key.is_some()
            && self.rows_key != Some((self.buf.revision(), self.vim.mode == Mode::Insert))
        {
            self.buf.rows.clear();
            self.rows_key = None;
        }
        // Shortcuts work in every mode, and do what the toolbar buttons do.
        match key {
            Key::Ctrl('s') => {
                self.save();
                return false;
            }
            // In normal mode Ctrl-b and Ctrl-u scroll instead.
            Key::Ctrl(c @ ('b' | 'i' | 'u')) if c == 'i' || self.vim.mode != Mode::Normal => {
                self.apply(&match c {
                    'b' => Action::Bold,
                    'i' => Action::Italic,
                    _ => Action::Underline,
                });
                return false;
            }
            _ => {}
        }
        let before = self.buf.cursor();
        let quit = match self.vim.handle(&mut self.buf, key) {
            Some(Command::Save) => {
                self.save();
                false
            }
            Some(Command::SaveQuit) => self.save(),
            Some(Command::Reload) => {
                self.reload();
                false
            }
            Some(Command::Quit | Command::ForceQuit) => true,
            Some(Command::Help) => {
                self.help = true;
                false
            }
            Some(Command::Scroll(by)) => {
                self.scroll = Some(by);
                false
            }
            None => false,
        };
        self.snap_cursor(before, key);
        quit
    }

    /// Outside insert mode the syntax is hidden, so the cursor may not rest on it. It moves on in
    /// the direction it was going. Vim counts a hidden `**` as a word of its own, so a word motion
    /// that lands on one goes on to the visible word: `w` steps over the blanks, `e` and `b`
    /// finish from the char it snapped to.
    fn snap_cursor(&mut self, before: usize, key: Key) {
        if self.vim.mode == Mode::Insert {
            return;
        }
        let cursor = self.buf.cursor();
        let (display, _) = self.display();
        let mut snapped = display.snap(cursor, cursor >= before);
        if snapped != cursor {
            let text = self.buf.text();
            let next = text[snapped..].chars().next();
            let blank = next.is_some_and(|c| c == ' ' || c == '\t');
            let target = match key {
                Key::Char('w') => {
                    let blanks = text[snapped..]
                        .bytes()
                        .take_while(|b| matches!(b, b' ' | b'\t'))
                        .count();
                    Some((snapped + blanks, true))
                }
                Key::Char('e') if blank => Some((word_end(text, snapped), true)),
                Key::Char('b') => Some((
                    word_back(text, snapped + next.map_or(0, char::len_utf8)),
                    false,
                )),
                _ => None,
            };
            if let Some((to, forward)) = target {
                snapped = self.display().0.snap(to, forward);
            }
        }
        if snapped != cursor {
            self.buf.set_cursor(snapped);
        }
    }

    /// Puts the cursor at displayed offset `display_offset`, as a click there would.
    pub fn click(&mut self, display_offset: usize) {
        let source = self.display().0.to_source(display_offset);
        self.buf.set_cursor(source);
        if self.vim.mode != Mode::Insert {
            self.snap_cursor(0, Key::Esc);
            self.buf.clamp_to_char();
        }
    }

    /// The display, with the cursor and selection put in its terms.
    pub fn view(&mut self) -> View<'_> {
        let cursor = self.buf.cursor();
        let selection = self.vim.selection(&self.buf);
        let mode = self.vim.mode;
        let version = self.display().1;
        // A `:s` being typed shows the text it would make, replacements lit, until Enter.
        let revision = self.buf.revision();
        match self.vim.command_line() {
            Some((':', typed)) => {
                if self.preview_for.0 != revision || self.preview_for.1 != typed {
                    self.preview_for = (revision, typed.to_string());
                    self.preview = self.vim.preview_substitute(&self.buf, typed).map(|made| {
                        let display = Display::normal(&made.text);
                        let found = made
                            .changes
                            .iter()
                            .map(|(_, new)| {
                                display.to_display(new.start)..display.to_display(new.end)
                            })
                            .filter(|range| !range.is_empty())
                            .collect();
                        Preview {
                            cursor: display.to_display(made.map_cursor(cursor)),
                            found,
                            version: NEXT_VERSION.fetch_add(1, Ordering::Relaxed),
                            display,
                        }
                    });
                }
            }
            _ => {
                self.preview = None;
                self.preview_for.1.clear();
            }
        }
        if let Some(preview) = &self.preview {
            return View {
                display: &preview.display,
                version: preview.version,
                mode,
                cursor: preview.cursor,
                selection: None,
                found: &preview.found,
            };
        }
        let display = &self.display;
        let pattern = self.vim.highlight_pattern();
        let pattern = pattern.as_deref().unwrap_or("");
        if self.found_for.0 != version || self.found_for.1 != pattern {
            self.found_for.0 = version;
            self.found_for.1.clear();
            self.found_for.1.push_str(pattern);
            self.found.clear();
            if !pattern.is_empty() {
                // A few thousand is more than anyone looks at, and keeps a one-letter search cheap.
                for (at, found) in self.buf.text().match_indices(pattern).take(20_000) {
                    let range = display.to_display(at)..display.to_display(at + found.len());
                    if !range.is_empty() {
                        self.found.push(range);
                    }
                }
            }
        }
        View {
            display,
            version,
            mode,
            cursor: display.to_display(cursor),
            selection: selection.map(|r| display.to_display(r.start)..display.to_display(r.end)),
            found: &self.found,
        }
    }

    /// Carries out a toolbar or shortcut action on the visual selection, or the word or line at the
    /// cursor. Each is one undo step. A new link leaves you typing its address.
    pub fn apply(&mut self, action: &Action) {
        let buf = &mut self.buf;
        match action {
            Action::Bold => self.vim.apply_format(buf, |b, r| toggle(b, r, Mark::Bold)),
            Action::Italic => self
                .vim
                .apply_format(buf, |b, r| toggle(b, r, Mark::Italic)),
            Action::Strikethrough => {
                self.vim
                    .apply_format(buf, |b, r| toggle(b, r, Mark::Strikethrough));
            }
            Action::Underline => self.vim.apply_format(buf, toggle_underline),
            Action::Uppercase => self.vim.apply_format(buf, uppercase),
            Action::Color(color) => self.vim.apply_format(buf, |b, r| set_color(b, r, *color)),
            Action::FontSize(pt) => self.vim.apply_format(buf, |b, r| set_font_size(b, r, *pt)),
            Action::FontFamily(name) => {
                self.vim
                    .apply_format(buf, |b, r| set_font_family(b, r, name));
            }
            Action::Heading(level) => self.vim.apply_format(buf, |b, r| set_heading(b, r, *level)),
            Action::BulletList => self.vim.apply_format(buf, |b, r| toggle_list(b, r, false)),
            Action::NumberedList => self.vim.apply_format(buf, |b, r| toggle_list(b, r, true)),
            Action::Quote => self.vim.apply_format(buf, toggle_quote),
            Action::Link => {
                let mut cursor = None;
                self.vim
                    .apply_format(buf, |b, r| cursor = toggle_link(b, r));
                if let Some(cursor) = cursor {
                    self.buf.set_cursor(cursor);
                    if self.vim.mode != Mode::Insert {
                        self.vim.insert_here(&mut self.buf);
                    }
                }
            }
        }
        self.snap_cursor(self.buf.cursor(), Key::Esc);
    }

    /// Reads the file again, replacing the text. It is one edit, so undo brings back what was
    /// there, and the buffer counts as saved. Does nothing without a file. On failure the status
    /// line says why and the text stays.
    pub fn reload(&mut self) {
        let Some(path) = self.doc.path().map(Path::to_path_buf) else {
            self.vim.message = "E32: No file name".into();
            return;
        };
        match Document::open(&path) {
            Ok(fresh) => {
                let len = self.buf.text().len();
                let cursor = self.buf.cursor();
                self.buf.replace(0..len, fresh.text());
                self.buf.mark_saved();
                self.doc = fresh;
                self.buf.set_cursor(cursor);
                self.snap_cursor(cursor, Key::Esc);
                self.buf.clamp_to_char();
            }
            Err(error) => self.vim.message = format!("E484: Can't read the file: {error}"),
        }
    }

    /// True if the file on disk no longer holds what was last read or written.
    pub fn changed_on_disk(&self) -> bool {
        self.doc
            .path()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .is_some_and(|disk| disk != self.doc.text())
    }

    /// The visual selection's text, for copying, or `None` outside visual mode.
    pub fn copy_selection(&self) -> Option<String> {
        let range = self.vim.selection(&self.buf)?;
        Some(self.buf.text()[range].to_string())
    }

    /// Copies the visual selection's text and deletes it.
    pub fn cut_selection(&mut self) -> Option<String> {
        let text = self.copy_selection()?;
        self.key(Key::Char('d'));
        Some(text)
    }

    /// Puts `text` into the document: at the cursor while inserting, in place of the selection in
    /// visual mode, and after the cursor's char otherwise, as `p` does. One undo step.
    pub fn paste_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let insert = self.vim.mode == Mode::Insert;
        if let Some(range) = self.vim.selection(&self.buf) {
            self.buf.begin_group();
            self.buf.replace(range.clone(), text);
            self.buf.end_group();
            self.key(Key::Esc);
            self.buf.set_cursor(range.start + text.len());
        } else if insert {
            self.buf.insert(text);
        } else {
            let after = self.buf.text()[self.buf.cursor()..]
                .chars()
                .next()
                .filter(|&c| c != '\n')
                .map_or(0, char::len_utf8);
            let at = self.buf.cursor() + after;
            self.buf.replace(at..at, text);
            // Like `p`: on the last char of a one-line paste, at the start of a longer one.
            let last = text.chars().next_back().map_or(0, char::len_utf8);
            self.buf.set_cursor(if text.contains('\n') {
                at
            } else {
                at + text.len() - last
            });
        }
        if !insert {
            self.buf.clamp_to_char();
            self.snap_cursor(self.buf.cursor(), Key::Esc);
        }
    }

    /// Writes the text to its file. This is what the Save button and `:w` both do. On failure it
    /// tells the status line why and returns false.
    pub fn save(&mut self) -> bool {
        self.doc.set_text(self.buf.text());
        match self.doc.save() {
            Ok(()) => {
                self.buf.mark_saved();
                true
            }
            Err(error) => {
                self.vim.message = match error.kind() {
                    io::ErrorKind::NotFound if self.doc.path().is_none() => {
                        "E32: No file name".into()
                    }
                    _ => format!("E212: Can't write the file: {error}"),
                };
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn type_keys(editor: &mut Editor, keys: &str) -> bool {
        let mut quit = false;
        for c in keys.chars() {
            quit |= editor.key(match c {
                '\n' => Key::Enter,
                '\x1b' => Key::Esc,
                '\x12' => Key::Ctrl('r'),
                c => Key::Char(c),
            });
        }
        quit
    }

    fn temp_file(name: &str, text: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("bwa_editor_{name}.dj"));
        std::fs::write(&path, text).unwrap();
        path
    }

    #[test]
    fn typing_edits_the_buffer_and_w_saves_the_file() {
        let path = temp_file("save", "one two");
        let mut editor = Editor::open(&path).unwrap();
        assert!(!type_keys(&mut editor, "dwiZ\x1b"));
        assert_eq!(editor.buf.text(), "Ztwo");
        assert!(editor.buf.is_modified());
        // Nothing reaches the disk until :w, and then the buffer counts as saved.
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "one two");
        assert!(!type_keys(&mut editor, ":w\n"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "Ztwo");
        assert!(!editor.buf.is_modified());
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn quit_needs_a_saved_buffer_unless_forced() {
        let path = temp_file("quit", "abc");
        let mut editor = Editor::open(&path).unwrap();
        assert!(type_keys(&mut editor, ":q\n"));
        type_keys(&mut editor, "x");
        assert!(!type_keys(&mut editor, ":q\n"));
        assert!(editor.vim.message.starts_with("E37"));
        assert!(type_keys(&mut editor, ":q!\n"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "abc");
        // :wq saves, then quits.
        assert!(type_keys(&mut editor, ":wq\n"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "bc");
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn a_save_that_fails_says_so_and_does_not_quit() {
        // No file to save to.
        let mut editor = Editor::default();
        type_keys(&mut editor, "ihi\x1b");
        assert!(!type_keys(&mut editor, ":wq\n"));
        assert_eq!(editor.vim.message, "E32: No file name");
        assert!(editor.buf.is_modified());
        // A file that can't be written.
        let path = temp_file("fail", "abc");
        let mut editor = Editor::open(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        type_keys(&mut editor, "x");
        assert!(!type_keys(&mut editor, ":wq\n"));
        assert!(
            editor.vim.message.starts_with("E212"),
            "{}",
            editor.vim.message
        );
        assert!(editor.buf.is_modified());
        std::fs::remove_dir(&path).unwrap();
    }

    #[test]
    fn the_display_is_rebuilt_only_when_the_text_changes() {
        let mut editor = Editor::default();
        let first = editor.display().1;
        assert_eq!(editor.display().1, first);
        type_keys(&mut editor, "l");
        assert_eq!(editor.display().1, first);
        type_keys(&mut editor, "i*hi*");
        // While inserting, the display is the source; after Esc it hides the syntax.
        let (display, version) = editor.display();
        assert_eq!(display.text, "*hi*");
        assert!(version > first);
        type_keys(&mut editor, "\x1b");
        let (display, version_after) = editor.display();
        assert_eq!(display.text, "hi");
        assert!(version_after > version);
    }

    #[test]
    fn the_cursor_skips_hidden_syntax_outside_insert_mode() {
        let mut editor = Editor::default();
        // a0 ' '1 *2 *3 b4 *5 *6 ' '7 c8 ' '9 d10
        type_keys(&mut editor, "ia **b** c d\x1b0");
        let at = |editor: &mut Editor, keys: &str| {
            type_keys(editor, keys);
            editor.buf.cursor()
        };
        assert_eq!(at(&mut editor, "w"), 4);
        // Right over the closing ** lands on the space, and left comes back to the b.
        assert_eq!(at(&mut editor, "l"), 7);
        assert_eq!(at(&mut editor, "h"), 4);
        // w from the b skips the ** and the space after it, to the next word.
        assert_eq!(at(&mut editor, "w"), 8);
        assert_eq!(at(&mut editor, "b"), 4);
        assert_eq!(at(&mut editor, "e"), 8);
        assert_eq!(at(&mut editor, "$"), 10);
        assert_eq!(at(&mut editor, "0"), 0);
        // Inserting doesn't snap: the raw text is showing.
        assert_eq!(at(&mut editor, "wa"), 5);
    }

    #[test]
    fn word_motions_treat_a_hidden_delimiter_run_as_part_of_the_word_next_to_it() {
        let mut editor = Editor::default();
        // Visible: "one bold two"
        type_keys(&mut editor, "ione **bold** two\x1b0");
        for (keys, want) in [
            ("w", 6),
            ("w", 13),
            ("b", 6),
            ("b", 0),
            ("we", 9),
            ("e", 15),
            ("ee", 15),
        ] {
            type_keys(&mut editor, keys);
            assert_eq!(editor.buf.cursor(), want, "after {keys}");
        }
    }

    #[test]
    fn clicking_places_the_cursor_on_the_source_char_that_was_shown() {
        let mut editor = Editor::default();
        type_keys(&mut editor, "i## Hi\n\na **b** c\x1b");
        // Displayed: "Hi\n\na b c"; the b is at 6 and comes from source offset 11.
        editor.click(6);
        assert_eq!(editor.buf.cursor(), 11);
        // Past the end lands on the last char, not after it.
        editor.click(99);
        assert_eq!(editor.buf.cursor(), 15);
        editor.click(0);
        assert_eq!(editor.buf.cursor(), 3);
        // While inserting, the raw text shows, so a click is a raw offset.
        type_keys(&mut editor, "i");
        editor.click(2);
        assert_eq!(editor.buf.cursor(), 2);
    }

    #[test]
    fn the_view_puts_the_cursor_and_selection_in_display_terms() {
        let mut editor = Editor::default();
        type_keys(&mut editor, "i**ab** cd\x1b0");
        let view = editor.view();
        assert_eq!((view.cursor, view.selection.clone()), (0, None));
        assert_eq!(view.display.text, "ab cd");
        type_keys(&mut editor, "ve");
        let view = editor.view();
        assert_eq!(
            (view.mode, view.cursor, view.selection),
            (Mode::Visual, 1, Some(0..2))
        );
        // Inserting shows the source, so the offsets are the source's.
        type_keys(&mut editor, "\x1bi");
        let view = editor.view();
        assert_eq!(
            (view.mode, view.cursor, view.selection),
            (Mode::Insert, 3, None)
        );
    }

    #[test]
    fn a_file_opens_with_the_cursor_on_visible_text() {
        let path = temp_file("open_cursor", "## Title\n");
        let editor = Editor::open(&path).unwrap();
        assert_eq!(editor.buf.cursor(), 3);
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn word_count_and_file_name() {
        let mut editor = Editor::default();
        assert_eq!(
            (editor.word_count(), editor.file_name().as_str()),
            (0, "[No Name]")
        );
        type_keys(&mut editor, "ione two\nthree\x1b");
        assert_eq!(editor.word_count(), 3);
        type_keys(&mut editor, "dd");
        assert_eq!(editor.word_count(), 2);
        let path = temp_file("name", "a b");
        let mut editor = Editor::open(&path).unwrap();
        assert_eq!(
            (editor.word_count(), editor.file_name().as_str()),
            (2, "bwa_editor_name.dj")
        );
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn a_new_editor_never_shares_a_display_version_with_an_old_one() {
        let (mut a, mut b) = (Editor::default(), Editor::default());
        assert_ne!(a.display().1, b.display().1);
    }

    #[test]
    fn the_view_lists_where_the_search_text_is_in_display_offsets() {
        let mut editor = Editor::default();
        type_keys(&mut editor, "ia **foo** b foo\x1b/foo\n");
        // The `**` is hidden, so the first match is at 2, not 4.
        assert_eq!(editor.view().found, [2..5, 8..11]);
        type_keys(&mut editor, ":noh\n");
        assert!(editor.view().found.is_empty());
    }

    #[test]
    fn the_current_font_is_the_one_at_the_cursor_or_shared_by_the_selection() {
        let mut editor = Editor::default();
        type_keys(&mut editor, "ione two three\x1b0w");
        assert_eq!(editor.current_font().as_deref(), Some("Serif"));
        editor.apply(&Action::FontFamily("Georgia".into()));
        assert_eq!(editor.current_font().as_deref(), Some("Georgia"));
        // On another word it is the body font again; across both it is none.
        type_keys(&mut editor, "w");
        assert_eq!(editor.current_font().as_deref(), Some("Serif"));
        type_keys(&mut editor, "bvee");
        assert_eq!(editor.current_font(), None);
        type_keys(&mut editor, "\x1b0wve");
        assert_eq!(editor.current_font().as_deref(), Some("Georgia"));
    }

    #[test]
    fn a_substitute_being_typed_shows_its_result_in_the_view() {
        let mut editor = Editor::default();
        type_keys(&mut editor, "ione two one\x1b0");
        // Nothing to show until there is a replacement, but the pattern is already lit.
        type_keys(&mut editor, ":%s/one");
        assert_eq!(editor.view().display.text, "one two one");
        assert_eq!(editor.view().found, [0..3, 8..11]);
        type_keys(&mut editor, "/uno/g");
        let view = editor.view();
        assert_eq!(view.display.text, "uno two uno");
        assert_eq!(view.found, [0..3, 8..11]);
        // The real text is untouched until Enter, and Esc drops the preview.
        assert_eq!(editor.buf.text(), "one two one");
        type_keys(&mut editor, "\x1b");
        assert_eq!(editor.view().display.text, "one two one");
        type_keys(&mut editor, ":%s/one/1/g\n");
        assert_eq!(editor.buf.text(), "1 two 1");
        assert_eq!(editor.view().display.text, "1 two 1");
    }

    #[test]
    fn headings_are_kept_until_the_text_changes_and_goto_lands_on_the_title() {
        let mut editor = Editor::default();
        type_keys(&mut editor, "i# One\ntext\n## Two\x1b");
        let found: Vec<(usize, String)> = editor
            .headings()
            .iter()
            .map(|h| (h.level, h.title.clone()))
            .collect();
        assert_eq!(found, [(1, "One".to_string()), (2, "Two".to_string())]);
        // In normal mode the `## ` is hidden, so the cursor rests on the T.
        editor.goto(12);
        assert_eq!(editor.buf.cursor(), 14);
        editor.goto(0);
        assert_eq!(editor.buf.cursor(), 2);
        type_keys(&mut editor, "dd");
        assert_eq!(editor.headings().len(), 1);
    }

    #[test]
    fn spot_is_the_word_under_the_cursor_or_the_words_selected() {
        let mut editor = Editor::default();
        assert_eq!(editor.spot(), Spot::Word(0));
        type_keys(&mut editor, "ione two  three\x1b0");
        assert_eq!(editor.spot(), Spot::Word(1));
        type_keys(&mut editor, "w");
        assert_eq!(editor.spot(), Spot::Word(2));
        // In the blanks after a word it is still that word.
        type_keys(&mut editor, "e");
        type_keys(&mut editor, "l");
        assert_eq!(editor.spot(), Spot::Word(2));
        type_keys(&mut editor, "0v$");
        assert_eq!(editor.spot(), Spot::Selected(3));
        type_keys(&mut editor, "\x1bwv");
        assert_eq!(editor.spot(), Spot::Selected(1));
    }

    #[test]
    fn up_and_down_follow_the_rows_the_window_wrapped_the_text_into() {
        let mut editor = Editor::default();
        type_keys(&mut editor, "ione two three\nfour\x1b0");
        // Before the window has laid anything out, whole lines.
        type_keys(&mut editor, "j");
        assert_eq!(editor.buf.cursor(), 14);
        type_keys(&mut editor, "k");
        // The window wrapped the first line after "two ".
        editor.view();
        editor.set_rows(&[0, 8, 14]);
        type_keys(&mut editor, "j");
        assert_eq!(editor.buf.cursor(), 8);
        type_keys(&mut editor, "j");
        assert_eq!(editor.buf.cursor(), 14);
        // An operator still works on whole lines.
        type_keys(&mut editor, "kdj");
        assert_eq!(editor.buf.text(), "");
        // Once the text changes, the old rows say nothing about it.
        type_keys(&mut editor, "ione two three\x1b0");
        assert!(editor.buf.rows.is_empty());
    }

    #[test]
    fn actions_format_the_selection_or_the_word_at_the_cursor() {
        let mut editor = Editor::default();
        type_keys(&mut editor, "ione two three\x1b0w");
        // No selection: the word at the cursor.
        editor.apply(&Action::Bold);
        assert_eq!(editor.buf.text(), "one *two* three");
        editor.apply(&Action::Bold);
        assert_eq!(editor.buf.text(), "one two three");
        // A selection, and the visual mode ends.
        type_keys(&mut editor, "ve");
        editor.apply(&Action::Color([0xcc, 0, 0]));
        assert_eq!(
            editor.buf.text(),
            "one [two]{style=\"color:#cc0000\"} three"
        );
        assert_eq!(editor.vim.mode, Mode::Normal);
        // Undo takes back one action at a time.
        editor.apply(&Action::Uppercase);
        assert_eq!(
            editor.buf.text(),
            "one [TWO]{style=\"color:#cc0000\"} three"
        );
        type_keys(&mut editor, "u");
        assert_eq!(
            editor.buf.text(),
            "one [two]{style=\"color:#cc0000\"} three"
        );
    }

    #[test]
    fn block_actions_work_on_the_cursors_line() {
        let mut editor = Editor::default();
        type_keys(&mut editor, "iTitle\nline one\nline two\x1b");
        editor.apply(&Action::BulletList);
        assert_eq!(editor.buf.text(), "Title\nline one\n\n- line two");
        editor.apply(&Action::Heading(2));
        assert_eq!(editor.buf.text(), "Title\nline one\n\n## - line two");
        type_keys(&mut editor, "gg");
        editor.apply(&Action::Heading(1));
        editor.apply(&Action::Quote);
        assert!(
            editor.buf.text().starts_with("> # Title"),
            "{}",
            editor.buf.text()
        );
    }

    #[test]
    fn a_new_link_leaves_you_typing_its_address() {
        let mut editor = Editor::default();
        type_keys(&mut editor, "isee word here\x1b0w");
        editor.apply(&Action::Link);
        assert_eq!(editor.buf.text(), "see [word]() here");
        assert_eq!(editor.vim.mode, Mode::Insert);
        type_keys(&mut editor, "http://x.y\x1b");
        assert_eq!(editor.buf.text(), "see [word](http://x.y) here");
        // The address and the wrapping are separate undo steps.
        type_keys(&mut editor, "u");
        assert_eq!(editor.buf.text(), "see [word]() here");
        type_keys(&mut editor, "u");
        assert_eq!(editor.buf.text(), "see word here");
        // Applied again inside a link, it comes off, and no insert starts.
        type_keys(&mut editor, "\x12\x120w");
        editor.apply(&Action::Link);
        assert_eq!(editor.buf.text(), "see word here");
        assert_eq!(editor.vim.mode, Mode::Normal);
    }

    #[test]
    fn shortcuts_format_and_save_in_any_mode() {
        let path = temp_file("shortcuts", "one two three");
        let mut editor = Editor::open(&path).unwrap();
        let ctrl = |editor: &mut Editor, c: char| editor.key(Key::Ctrl(c));
        // Ctrl+B over a visual selection. (In normal mode Ctrl+B and Ctrl+U scroll.)
        type_keys(&mut editor, "wviw");
        ctrl(&mut editor, 'b');
        assert_eq!(editor.buf.text(), "one *two* three");
        type_keys(&mut editor, "wve");
        ctrl(&mut editor, 'i');
        assert_eq!(editor.buf.text(), "one *two* _three_");
        assert_eq!(editor.vim.mode, Mode::Normal);
        type_keys(&mut editor, "0viw");
        ctrl(&mut editor, 'u');
        assert_eq!(
            editor.buf.text(),
            "[one]{style=\"text-decoration:underline\"} *two* _three_"
        );
        // While inserting, and Ctrl+S without leaving insert mode.
        type_keys(&mut editor, "A!");
        ctrl(&mut editor, 's');
        assert_eq!(editor.vim.mode, Mode::Insert);
        assert!(!editor.buf.is_modified());
        assert!(std::fs::read_to_string(&path).unwrap().ends_with("_!"));
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn opening_a_missing_file_is_an_error() {
        assert!(Editor::open(Path::new("/nonexistent/dir/book.dj")).is_err());
    }

    #[test]
    fn reload_takes_the_files_text_and_can_be_undone() {
        let path = temp_file("reload", "one two");
        let mut editor = Editor::open(&path).unwrap();
        type_keys(&mut editor, "wx");
        assert!(editor.buf.is_modified() && !editor.changed_on_disk());
        std::fs::write(&path, "new text on disk").unwrap();
        assert!(editor.changed_on_disk());
        assert!(!type_keys(&mut editor, ":e!\n"));
        assert_eq!(editor.buf.text(), "new text on disk");
        assert!(!editor.buf.is_modified() && !editor.changed_on_disk());
        type_keys(&mut editor, "u");
        assert_eq!(editor.buf.text(), "one wo");
        // Our own saves are not changes from outside.
        type_keys(&mut editor, ":w\n");
        assert!(!editor.changed_on_disk());
        // With no file, or a file that has gone, it says so and keeps the text.
        let mut nameless = Editor::default();
        type_keys(&mut nameless, "ihi\x1b:e!\n");
        assert_eq!(nameless.vim.message, "E32: No file name");
        std::fs::remove_file(&path).unwrap();
        type_keys(&mut editor, ":e!\n");
        assert!(
            editor.vim.message.starts_with("E484"),
            "{}",
            editor.vim.message
        );
        assert_eq!(editor.buf.text(), "one wo");
    }

    #[test]
    fn copy_cut_and_paste_text() {
        let mut editor = Editor::default();
        type_keys(&mut editor, "ione two three\x1b0");
        assert_eq!(editor.copy_selection(), None);
        type_keys(&mut editor, "ve");
        assert_eq!(editor.copy_selection().as_deref(), Some("one"));
        assert_eq!(editor.cut_selection().as_deref(), Some("one"));
        assert_eq!(editor.buf.text(), " two three");
        assert_eq!(editor.vim.mode, Mode::Normal);
        // Normal mode pastes after the cursor char; the cursor lands on the end of the text.
        editor.paste_text("ONE");
        assert_eq!(editor.buf.text(), " ONEtwo three");
        assert_eq!(editor.buf.cursor(), 3);
        type_keys(&mut editor, "u");
        assert_eq!(editor.buf.text(), " two three");
        // A selection is replaced, as one step, and insert mode pastes at the cursor.
        type_keys(&mut editor, "wviw");
        editor.paste_text("2");
        assert_eq!(editor.buf.text(), " 2 three");
        type_keys(&mut editor, "u");
        assert_eq!(editor.buf.text(), " two three");
        type_keys(&mut editor, "A");
        editor.paste_text(" and four");
        type_keys(&mut editor, "!\x1b");
        assert_eq!(editor.buf.text(), " two three and four!");
        // Empty text is nothing, and multi-line text is fine.
        editor.paste_text("");
        type_keys(&mut editor, "0");
        editor.paste_text("a\nb");
        assert_eq!(editor.buf.text(), " a\nbtwo three and four!");
    }
}
