use std::io;
use std::path::{Path, PathBuf};

/// A text document backed by an optional file on disk.
#[derive(Debug, Default)]
pub struct Document {
    text: String,
    path: Option<PathBuf>,
}

impl Document {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        let text = std::fs::read_to_string(&path)?;
        Ok(Self {
            text,
            path: Some(path),
        })
    }

    pub fn save(&self) -> io::Result<()> {
        match &self.path {
            Some(path) => std::fs::write(path, &self.text),
            None => Err(io::Error::new(
                io::ErrorKind::NotFound,
                "document has no path",
            )),
        }
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn set_text(&mut self, text: impl Into<String>) {
        self.text = text.into();
    }

    pub fn word_count(&self) -> usize {
        self.text.split_whitespace().count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_words_across_whitespace() {
        let mut doc = Document::new();
        doc.set_text("It was  a dark\nand stormy\tnight.");
        assert_eq!(doc.word_count(), 7);
    }

    #[test]
    fn empty_document_has_no_words() {
        assert_eq!(Document::new().word_count(), 0);
    }

    #[test]
    fn save_without_path_fails() {
        assert!(Document::new().save().is_err());
    }

    #[test]
    fn round_trips_through_disk() {
        let path = std::env::temp_dir().join("bwa_document_round_trip.txt");
        std::fs::write(&path, "Chapter one").unwrap();
        let mut doc = Document::open(&path).unwrap();
        assert_eq!(doc.text(), "Chapter one");
        doc.set_text("Chapter two");
        doc.save().unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "Chapter two");
        std::fs::remove_file(&path).unwrap();
    }
}
