use std::fs;
use std::path::PathBuf;

use anyhow::{bail, Context};
use serde::{Deserialize, Serialize};

use crate::args::Provider;

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct Config {
    pub default_provider: Option<String>,
    pub default_model: Option<String>,
    #[serde(default)]
    pub groq: ProviderConfig,
    #[serde(default)]
    pub gemini: ProviderConfig,
    #[serde(default)]
    pub ollama: ProviderConfig,
}

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct ProviderConfig {
    pub api_key: Option<String>,
}

impl Config {
    /// Load config from `~/.config/granite/config.toml` (or `$XDG_CONFIG_HOME`).
    /// Missing file yields defaults — no config file is not an error.
    pub fn load() -> anyhow::Result<Self> {
        let path = config_path()?;
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = fs::read_to_string(&path)
            .with_context(|| format!("failed to read config file at {}", path.display()))?;
        toml::from_str(&raw)
            .with_context(|| format!("failed to parse config file at {}", path.display()))
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let path = config_path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create config dir at {}", parent.display()))?;
        }
        let raw = toml::to_string_pretty(self).context("failed to serialize config")?;
        fs::write(&path, raw)
            .with_context(|| format!("failed to write config file at {}", path.display()))
    }

    pub fn api_key_for(&self, provider: Provider) -> Option<&str> {
        match provider {
            Provider::Groq => self.groq.api_key.as_deref(),
            Provider::Gemini => self.gemini.api_key.as_deref(),
            Provider::Ollama => self.ollama.api_key.as_deref(),
        }
        .filter(|key| !key.is_empty())
    }

    pub fn set(&mut self, key: &str, value: &str) -> anyhow::Result<()> {
        match key {
            "default_provider" => self.default_provider = Some(value.to_string()),
            "default_model" => self.default_model = Some(value.to_string()),
            "groq.api_key" => self.groq.api_key = Some(value.to_string()),
            "gemini.api_key" => self.gemini.api_key = Some(value.to_string()),
            "ollama.api_key" => self.ollama.api_key = Some(value.to_string()),
            other => bail!(
                "unknown config key '{other}' — expected one of: default_provider, default_model, groq.api_key, gemini.api_key, ollama.api_key"
            ),
        }
        Ok(())
    }
}

fn config_path() -> anyhow::Result<PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        return Ok(PathBuf::from(xdg).join("granite/config.toml"));
    }
    let home = std::env::var("HOME").context("neither XDG_CONFIG_HOME nor HOME is set")?;
    Ok(PathBuf::from(home).join(".config/granite/config.toml"))
}

/// Mask a secret for display: keep the first 4 chars, replace the rest with `*`.
pub fn mask(secret: &str) -> String {
    if secret.len() <= 4 {
        "*".repeat(secret.len())
    } else {
        format!("{}{}", &secret[..4], "*".repeat(secret.len() - 4))
    }
}
