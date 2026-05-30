use crate::error::{Result, ScanevmError};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum OutputFormat {
    #[default]
    Table,
    Json,
}

impl std::str::FromStr for OutputFormat {
    type Err = ScanevmError;
    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "table" => Ok(OutputFormat::Table),
            "json" => Ok(OutputFormat::Json),
            _ => Err(ScanevmError::Config(format!(
                "invalid output format: '{s}'"
            ))),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(default)]
    pub default_output: OutputFormat,
}

impl Config {
    pub fn path() -> PathBuf {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".scanevm")
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
            // The directory holds the plaintext API key — keep it owner-only.
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700));
            }
        }
        let json = serde_json::to_string_pretty(self)?;
        Self::write_private(&path, json.as_bytes())?;
        Ok(())
    }

    /// Write `contents` to `path` so the file is never world-readable.
    ///
    /// On Unix the file is created with mode 0600 (and re-chmod'd in case it
    /// already existed with looser permissions) — it stores the API key in
    /// plaintext, so it must not be readable by other local users.
    fn write_private(path: &PathBuf, contents: &[u8]) -> Result<()> {
        #[cfg(unix)]
        {
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(path)?;
            f.write_all(contents)?;
            // `mode` only applies on creation; enforce it if the file pre-existed.
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
        }
        #[cfg(not(unix))]
        {
            std::fs::write(path, contents)?;
        }
        Ok(())
    }

    pub fn require_api_key(&self) -> Result<String> {
        self.api_key.clone().ok_or(ScanevmError::ApiKeyMissing)
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
        let cfg = Config {
            api_key: Some("abc".into()),
            ..Default::default()
        };
        assert_eq!(cfg.require_api_key().unwrap(), "abc");
    }
}
