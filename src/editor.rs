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
use crate::vim::{Command, Key, Mode, Vim};
use std::io;
use std::ops::Range;
use std::path::Path;

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
    /// Counts the times `display` was rebuilt, so a renderer can tell when it changed.
    display_version: u64,
    /// The word count and the buffer revision it was counted at.
    words: Option<(u64, usize)>,
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
            self.display_version += 1;
        }
        (&self.display, self.display_version)
    }

    /// Feeds a key to the vim engine and carries out what it asks for. True means the app
    /// should quit.
    pub fn key(&mut self, key: Key) -> bool {
        let before = self.buf.cursor();
        let quit = match self.vim.handle(&mut self.buf, key) {
            Some(Command::Save) => {
                self.save();
                false
            }
            Some(Command::SaveQuit) => self.save(),
            Some(Command::Quit | Command::ForceQuit) => true,
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
        let (display, version) = self.display();
        View {
            display,
            version,
            mode,
            cursor: display.to_display(cursor),
            selection: selection.map(|r| display.to_display(r.start)..display.to_display(r.end)),
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
    fn opening_a_missing_file_is_an_error() {
        assert!(Editor::open(Path::new("/nonexistent/dir/book.dj")).is_err());
    }
}
