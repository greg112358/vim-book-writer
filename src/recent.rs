use serde::{Deserialize, Serialize};
use std::io;
use std::path::{Path, PathBuf};

const CAP: usize = 20;

/// The files opened lately, newest first, for the start screen's "Open recent".
#[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Recent {
    files: Vec<PathBuf>,
}

impl Recent {
    pub fn files(&self) -> &[PathBuf] {
        &self.files
    }

    /// Puts `path` at the front. If it was already listed it moves there, and only the 20 newest
    /// are kept. Paths compare as given, so pass absolute ones.
    pub fn add(&mut self, path: impl Into<PathBuf>) {
        let path = path.into();
        self.files.retain(|p| *p != path);
        self.files.insert(0, path);
        self.files.truncate(CAP);
    }

    /// Reads the list from a TOML file. A missing or unreadable file gives an empty list, since
    /// losing the recent files is better than refusing to start.
    pub fn load(path: &Path) -> Self {
        let mut recent: Self = std::fs::read_to_string(path)
            .ok()
            .and_then(|text| toml::from_str(&text).ok())
            .unwrap_or_default();
        recent.files.truncate(CAP);
        recent
    }

    /// Writes the list as TOML, creating the directory if needed.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        let text = toml::to_string(self).map_err(io::Error::other)?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newest_first_deduped_and_capped_at_twenty() {
        let mut recent = Recent::default();
        for name in ["a", "b", "c"] {
            recent.add(name);
        }
        assert_eq!(recent.files(), ["c", "b", "a"].map(PathBuf::from));
        // Opening one again moves it to the front instead of listing it twice.
        recent.add("a");
        assert_eq!(recent.files(), ["a", "c", "b"].map(PathBuf::from));

        for i in 0..25 {
            recent.add(format!("file{i}"));
        }
        assert_eq!(recent.files().len(), 20);
        assert_eq!(recent.files()[0], PathBuf::from("file24"));
        assert_eq!(recent.files()[19], PathBuf::from("file5"));
        // A file that fell off can come back, and a listed one moves up without growing the list.
        recent.add("a");
        recent.add("file10");
        assert_eq!(recent.files().len(), 20);
        assert_eq!(recent.files()[..2], ["file10", "a"].map(PathBuf::from));
    }

    #[test]
    fn persists_to_toml_and_survives_bad_files() {
        let dir = std::env::temp_dir().join("bwa_recent");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("nested/recent.toml");

        let mut recent = Recent::default();
        recent.add("/books/one.dj");
        recent.add("/books/two.dj");
        recent.save(&path).unwrap();
        assert_eq!(Recent::load(&path), recent);

        // Missing, corrupt, and wrongly typed files give an empty list.
        assert_eq!(Recent::load(&dir.join("missing.toml")), Recent::default());
        std::fs::write(&path, "files = [").unwrap();
        assert_eq!(Recent::load(&path), Recent::default());
        std::fs::write(&path, "files = 3").unwrap();
        assert_eq!(Recent::load(&path), Recent::default());
        // A hand-edited list longer than the cap is trimmed.
        let long: Vec<String> = (0..30).map(|i| format!("\"f{i}\"")).collect();
        std::fs::write(&path, format!("files = [{}]", long.join(","))).unwrap();
        assert_eq!(Recent::load(&path).files().len(), 20);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
