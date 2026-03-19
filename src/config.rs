use std::{fs, path::PathBuf};

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
}

impl Default for AppConfig {
    fn default() -> Self {
        let model_path = default_model_path();
        Self {
            autopaste: false,
            language: None,
            model_path,
            hotkey: crate::hotkey::default_hotkey().to_string(),
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
        fs::write(&path, raw)
            .with_context(|| format!("failed to write config file at {}", path.display()))?;

        Ok(())
    }
}

fn default_model_path() -> PathBuf {
    if let Some(project_dirs) = ProjectDirs::from(APP_QUALIFIER, APP_ORG, APP_NAME) {
        project_dirs.data_local_dir().join("models/ggml-base.en.bin")
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
    use super::AppConfig;

    #[test]
    fn default_config_has_hotkey() {
        let cfg = AppConfig::default();
        assert!(!cfg.hotkey.trim().is_empty());
        assert!(!cfg.autopaste);
    }

    #[test]
    fn missing_hotkey_field_uses_default() {
        let raw = r#"
autopaste = true
model_path = "/tmp/model.bin"
"#;
        let parsed: AppConfig = toml::from_str(raw).expect("config should parse");
        assert!(!parsed.hotkey.trim().is_empty());
    }
}
