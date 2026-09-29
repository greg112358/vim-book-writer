use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub font_family: String,
    pub font_size: f32,
    pub vim_mode: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            font_family: "serif".into(),
            font_size: 16.0,
            vim_mode: false,
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
    fn rejects_wrong_types() {
        assert!(Config::from_toml("font_size = \"big\"").is_err());
    }
}
