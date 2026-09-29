use crate::buffer::Buffer;
use crate::display::Display;
use crate::document::Document;
use crate::vim::{Command, Key, Mode, Vim};
use std::io;
use std::path::Path;

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
}

impl Editor {
    pub fn open(path: &Path) -> io::Result<Self> {
        let doc = Document::open(path)?;
        Ok(Self {
            buf: Buffer::new(doc.text()),
            vim: Vim::new(),
            doc,
            display: Display::default(),
            display_key: None,
            display_version: 0,
        })
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
        match self.vim.handle(&mut self.buf, key) {
            Some(Command::Save) => {
                self.save();
                false
            }
            Some(Command::SaveQuit) => self.save(),
            Some(Command::Quit | Command::ForceQuit) => true,
            None => false,
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
    fn opening_a_missing_file_is_an_error() {
        assert!(Editor::open(Path::new("/nonexistent/dir/book.dj")).is_err());
    }
}
