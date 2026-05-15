use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub hotkey: String,
    pub whisper_model: String,
    pub whisper_device: String,
    // ISO-639-1 code (e.g. "en", "fr"). Use "auto" to let whisper.cpp detect
    // the spoken language per utterance — only meaningful with multilingual
    // models; `.en`-suffix models are English-only regardless of this setting.
    #[serde(default = "default_language")]
    pub language: String,
    pub autostart: bool,
}

fn default_language() -> String {
    "en".into()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            hotkey: "shift+space".into(),
            whisper_model: "base.en-q5_1".into(),
            whisper_device: "cpu".into(),
            language: default_language(),
            autostart: true,
        }
    }
}

impl Config {
    pub fn load_or_default() -> Result<Self> {
        let path = config_path()?;
        if path.exists() {
            let raw = fs::read_to_string(&path)
                .with_context(|| format!("reading {}", path.display()))?;
            let cfg: Config = serde_json::from_str(&raw)
                .with_context(|| format!("parsing {}", path.display()))?;
            Ok(cfg)
        } else {
            let cfg = Config::default();
            cfg.save()?;
            Ok(cfg)
        }
    }

    pub fn save(&self) -> Result<()> {
        let path = config_path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let raw = serde_json::to_string_pretty(self)?;
        fs::write(&path, raw)?;
        Ok(())
    }
}

pub fn app_data_dir() -> Result<PathBuf> {
    let base = dirs::data_dir().context("no platform data dir")?;
    Ok(base.join("WhisperFreeFlow"))
}

pub fn models_dir() -> Result<PathBuf> {
    Ok(app_data_dir()?.join("models"))
}

fn config_path() -> Result<PathBuf> {
    Ok(app_data_dir()?.join("config.json"))
}
