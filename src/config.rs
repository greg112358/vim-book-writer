use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// The app's folder in the OS config location, for settings and the recent files list. `None` if
/// the home directory can't be found.
pub fn dir() -> Option<PathBuf> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
    let base = if cfg!(target_os = "macos") {
        home.join("Library/Application Support")
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| home.join(".config"))
    };
    Some(base.join("vim-book-writer"))
}

/// The app was once called `book-writing-app`. Moves its settings and recent files to the folder
/// for the new name, once, so nothing is lost by the rename.
pub fn migrate() {
    let Some(new) = dir() else { return };
    let old = new.with_file_name("book-writing-app");
    if !new.exists() && old.is_dir() {
        let _ = std::fs::rename(old, new);
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// The font the text is read in. This is how it looks to you, not part of the document.
    pub font: String,
    /// The size the text is read in, in points.
    pub size: f32,
    pub vim_mode: bool,
    /// Save the file by itself once edits stop.
    pub auto_save: bool,
    /// The dark look instead of the light one.
    pub dark: bool,
    /// The outline of headings is showing on the left.
    pub outline: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            font: "Serif".into(),
            size: 14.0,
            vim_mode: false,
            auto_save: false,
            dark: false,
            outline: true,
        }
    }
}

impl Config {
    pub fn from_toml(source: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(source)
    }

    pub fn to_toml(&self) -> Result<String, toml::ser::Error> {
        toml::to_string(self)
    }

    /// Reads the settings file. A missing or unreadable one gives the defaults, so a bad file
    /// never stops the app from starting.
    pub fn load(path: &std::path::Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| Self::from_toml(&text).ok())
            .unwrap_or_default()
    }

    /// Writes the settings file, making its folder if needed.
    pub fn save(&self, path: &std::path::Path) -> std::io::Result<()> {
        let text = self.to_toml().map_err(std::io::Error::other)?;
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
    fn missing_fields_fall_back_to_defaults() {
        let config = Config::from_toml("size = 20.0").unwrap();
        assert_eq!(config.size, 20.0);
        assert_eq!(config.font, "Serif");
        // Settings from before the reading font was one are ignored, not an error.
        let old = Config::from_toml("font_family = \"serif\"\nfont_size = 16.0").unwrap();
        assert_eq!((old.font.as_str(), old.size), ("Serif", 14.0));
        assert!(!config.vim_mode);
    }

    #[test]
    fn round_trips_through_toml() {
        let config = Config {
            vim_mode: true,
            ..Config::default()
        };
        assert_eq!(
            Config::from_toml(&config.to_toml().unwrap()).unwrap(),
            config
        );
    }

    #[test]
    fn loads_and_saves_the_file_and_forgives_a_bad_one() {
        let dir = std::env::temp_dir().join("bwa_config_file");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("nested/config.toml");
        assert_eq!(Config::load(&path), Config::default());
        let config = Config {
            auto_save: true,
            size: 20.0,
            ..Config::default()
        };
        config.save(&path).unwrap();
        assert_eq!(Config::load(&path), config);
        std::fs::write(&path, "auto_save = [").unwrap();
        assert_eq!(Config::load(&path), Config::default());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn rejects_wrong_types() {
        assert!(Config::from_toml("size = \"big\"").is_err());
    }
}
