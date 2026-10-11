use std::{fs, io::Write, path::PathBuf};

use anyhow::{Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

const APP_QUALIFIER: &str = "com";
const APP_ORG: &str = "vdora";
const APP_NAME: &str = "vdora";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub autopaste: bool,
    pub language: Option<String>,
    pub model_path: PathBuf,
    pub hotkey: String,
    pub log_level: LogLevel,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Info,
    Debug,
}

impl LogLevel {
    pub fn as_filter_directive(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Debug => "debug",
        }
    }

    pub fn as_ui_label(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Debug => "debug",
        }
    }
}

impl Default for LogLevel {
    fn default() -> Self {
        Self::Info
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        let model_path = default_model_path();
        Self {
            autopaste: false,
            language: None,
            model_path,
            hotkey: crate::hotkey::default_hotkey().to_string(),
            log_level: LogLevel::default(),
        }
    }
}

impl AppConfig {
    pub fn load_or_default() -> Self {
        match Self::load() {
            Ok(config) => config,
            Err(err) => {
                tracing::warn!("failed to load config, using defaults: {err}");
                Self::default()
            }
        }
    }

    pub fn load() -> Result<Self> {
        let path = config_file_path()?;
        if !path.exists() {
            return Ok(Self::default());
        }

        let raw = fs::read_to_string(&path)
            .with_context(|| format!("failed to read config file at {}", path.display()))?;
        let parsed = toml::from_str::<Self>(&raw).context("failed to parse config toml")?;

        Ok(parsed)
    }

    pub fn save(&self) -> Result<()> {
        let path = config_file_path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("failed to create config directory at {}", parent.display())
            })?;
        }

        let raw = toml::to_string_pretty(self).context("failed to serialize config")?;
        let parent = path
            .parent()
            .context("config path has no parent directory")?;
        let mut file = tempfile::NamedTempFile::new_in(parent)
            .context("failed to create temporary config file")?;
        file.write_all(raw.as_bytes())
            .context("failed to write config")?;
        file.as_file().sync_all().context("failed to sync config")?;
        file.persist(&path)
            .map_err(|err| err.error)
            .with_context(|| format!("failed to replace config file at {}", path.display()))?;

        Ok(())
    }
}

fn default_model_path() -> PathBuf {
    if let Some(project_dirs) = ProjectDirs::from(APP_QUALIFIER, APP_ORG, APP_NAME) {
        project_dirs
            .data_local_dir()
            .join("models/ggml-base.en.bin")
    } else {
        PathBuf::from("./models/ggml-base.en.bin")
    }
}

fn config_file_path() -> Result<PathBuf> {
    let project_dirs = ProjectDirs::from(APP_QUALIFIER, APP_ORG, APP_NAME)
        .context("could not determine user config directory")?;
    Ok(project_dirs.config_dir().join("config.toml"))
}

#[cfg(test)]
mod tests {
    use super::{AppConfig, LogLevel};

    #[test]
    fn config_round_trips_with_quoted_model_path() {
        let config = AppConfig {
            autopaste: true,
            language: Some("es".into()),
            model_path: std::path::PathBuf::from("/tmp/a \"quoted\" model.bin"),
            hotkey: "<Ctrl><Alt>space".into(),
            log_level: LogLevel::Debug,
        };
        let raw = toml::to_string_pretty(&config).unwrap();
        let parsed: AppConfig = toml::from_str(&raw).unwrap();
        assert_eq!(parsed.autopaste, config.autopaste);
        assert_eq!(parsed.language, config.language);
        assert_eq!(parsed.model_path, config.model_path);
        assert_eq!(parsed.hotkey, config.hotkey);
        assert_eq!(parsed.log_level, config.log_level);
    }

    #[test]
    fn default_config_has_hotkey() {
        let cfg = AppConfig::default();
        assert!(!cfg.hotkey.trim().is_empty());
        assert!(!cfg.autopaste);
        assert_eq!(cfg.log_level, LogLevel::Info);
    }

    #[test]
    fn missing_hotkey_field_uses_default() {
        let raw = r#"
autopaste = true
model_path = "/tmp/model.bin"
"#;
        let parsed: AppConfig = toml::from_str(raw).expect("config should parse");
        assert!(!parsed.hotkey.trim().is_empty());
        assert_eq!(parsed.log_level, LogLevel::Info);
    }

    #[test]
    fn parses_debug_log_level() {
        let raw = r#"
autopaste = false
model_path = "/tmp/model.bin"
log_level = "debug"
"#;
        let parsed: AppConfig = toml::from_str(raw).expect("config should parse");
        assert_eq!(parsed.log_level, LogLevel::Debug);
    }
}
