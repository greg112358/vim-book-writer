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

    /// Points the document at another file, or at none, without touching the disk.
    pub fn set_path(&mut self, path: Option<PathBuf>) {
        self.path = path;
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

