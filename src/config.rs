use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use crate::error::{AppError, Result};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum OutputFormat {
    Table,
    Json,
}

impl Default for OutputFormat {
    fn default() -> Self {
        OutputFormat::Table
    }
}

impl std::str::FromStr for OutputFormat {
    type Err = AppError;
    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "table" => Ok(OutputFormat::Table),
            "json" => Ok(OutputFormat::Json),
            _ => Err(AppError::Config(format!("invalid output format: '{s}'"))),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(default)]
    pub default_output: OutputFormat,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            api_key: None,
            default_output: OutputFormat::default(),
        }
    }
}

impl Config {
    pub fn path() -> PathBuf {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".etherscan-cli")
            .join("config.json")
    }

    pub fn load() -> Self {
        let mut cfg = Self::load_file().unwrap_or_default();
        // ETHERSCAN_API_KEY env var overrides config file
        if let Ok(key) = std::env::var("ETHERSCAN_API_KEY") {
            if !key.is_empty() {
                cfg.api_key = Some(key);
            }
        }
        cfg
    }

    fn load_file() -> Option<Self> {
        let path = Self::path();
        let contents = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&contents).ok()
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    pub fn require_api_key(&self) -> Result<String> {
        self.api_key.clone().ok_or(AppError::NoApiKey)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config() {
        let cfg = Config::default();
        assert_eq!(cfg.default_output, OutputFormat::Table);
        assert!(cfg.api_key.is_none());
    }

    #[test]
    fn require_api_key_missing() {
        let cfg = Config::default();
        assert!(cfg.require_api_key().is_err());
    }

    #[test]
    fn require_api_key_present() {
        let cfg = Config { api_key: Some("abc".into()), ..Default::default() };
        assert_eq!(cfg.require_api_key().unwrap(), "abc");
    }
}
