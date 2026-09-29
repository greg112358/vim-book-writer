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
    Some(base.join("book-writing-app"))
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub font_family: String,
    pub font_size: f32,
    pub vim_mode: bool,
    /// Save the file by itself once edits stop.
    pub auto_save: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            font_family: "serif".into(),
            font_size: 16.0,
            vim_mode: false,
            auto_save: false,
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
        let config = Config::from_toml("font_size = 20.0").unwrap();
        assert_eq!(config.font_size, 20.0);
        assert_eq!(config.font_family, "serif");
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
            font_size: 20.0,
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
        assert!(Config::from_toml("font_size = \"big\"").is_err());
    }
}
