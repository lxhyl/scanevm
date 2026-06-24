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
    /// Legacy single-key field. Still read from older config files and folded
    /// into the pool by [`Config::keys`]; [`Config::save`] consolidates it into
    /// `api_keys` and drops it, so new files only carry the array.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// The API key pool. Requests round-robin across these keys, each throttled
    /// independently, so N keys sustain roughly N× Etherscan's per-key rate
    /// limit (5 req/s on the free tier).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub api_keys: Vec<String>,
    #[serde(default)]
    pub default_output: OutputFormat,
    /// Set from the `--no-cache` CLI flag; never read from or written to disk.
    #[serde(skip)]
    pub no_cache: bool,
}

impl Config {
    pub fn path() -> PathBuf {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".scanevm")
            .join("config.json")
    }

    /// Load the on-disk config, then let the environment override the key pool.
    ///
    /// `ETHERSCAN_API_KEYS` (plural) is preferred for a pool; the legacy
    /// `ETHERSCAN_API_KEY` is still honored and also accepts a comma- or
    /// whitespace-separated list. Either env var, when non-empty, *replaces* the
    /// file's keys (matching the previous "env wins" behavior).
    pub fn load() -> Self {
        let mut cfg = Self::from_file();
        let env = std::env::var("ETHERSCAN_API_KEYS")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| {
                std::env::var("ETHERSCAN_API_KEY")
                    .ok()
                    .filter(|s| !s.trim().is_empty())
            });
        if let Some(raw) = env {
            let keys = dedup_keys(split_keys(&raw));
            if !keys.is_empty() {
                cfg.api_keys = keys;
                cfg.api_key = None;
            }
        }
        cfg
    }

    /// The config exactly as stored on disk, ignoring any environment override.
    /// Mutating commands (`config set-key`, …) start from this so they never
    /// persist a transient `ETHERSCAN_API_KEY` into the file.
    pub fn from_file() -> Self {
        Self::load_file().unwrap_or_default()
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
            // The directory holds the plaintext API keys — keep it owner-only.
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700));
            }
        }
        // Consolidate into the canonical `api_keys` array (deduped, legacy key
        // folded in) so the written file never carries both representations.
        let out = Config {
            api_key: None,
            api_keys: self.keys(),
            default_output: self.default_output.clone(),
            no_cache: false,
        };
        let json = serde_json::to_string_pretty(&out)?;
        Self::write_private(&path, json.as_bytes())?;
        Ok(())
    }

    /// Write `contents` to `path` so the file is never world-readable.
    ///
    /// On Unix the file is created with mode 0600 (and re-chmod'd in case it
    /// already existed with looser permissions) — it stores API keys in
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

    /// The effective API key pool: `api_keys` with the legacy single `api_key`
    /// folded in, blanks dropped, and duplicates removed (first occurrence
    /// wins, preserving order).
    pub fn keys(&self) -> Vec<String> {
        let mut all = self.api_keys.clone();
        if let Some(k) = &self.api_key {
            all.push(k.clone());
        }
        dedup_keys(all.into_iter().filter(|k| !k.trim().is_empty()).collect())
    }

    /// The key pool, or [`ScanevmError::ApiKeyMissing`] if it is empty.
    pub fn require_keys(&self) -> Result<Vec<String>> {
        let keys = self.keys();
        if keys.is_empty() {
            Err(ScanevmError::ApiKeyMissing)
        } else {
            Ok(keys)
        }
    }
}

/// Split a raw string into individual keys on commas/whitespace/semicolons,
/// trimming each and dropping blanks.
pub fn split_keys(raw: &str) -> Vec<String> {
    raw.split([',', ';', ' ', '\t', '\n', '\r'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}

/// Remove duplicate keys, keeping the first occurrence (stable order).
pub fn dedup_keys(keys: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    keys.into_iter().filter(|k| seen.insert(k.clone())).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config() {
        let cfg = Config::default();
        assert_eq!(cfg.default_output, OutputFormat::Table);
        assert!(cfg.keys().is_empty());
    }

    #[test]
    fn require_keys_missing() {
        let cfg = Config::default();
        assert!(cfg.require_keys().is_err());
    }

    #[test]
    fn legacy_single_key_is_folded_into_pool() {
        let cfg = Config {
            api_key: Some("legacy".into()),
            ..Default::default()
        };
        assert_eq!(cfg.require_keys().unwrap(), vec!["legacy".to_string()]);
    }

    #[test]
    fn pool_merges_legacy_and_array_and_dedupes() {
        let cfg = Config {
            api_key: Some("a".into()),
            api_keys: vec!["b".into(), "a".into(), "c".into()],
            ..Default::default()
        };
        // array first, legacy appended, duplicate "a" removed, order preserved.
        assert_eq!(cfg.keys(), vec!["b", "a", "c"]);
    }

    #[test]
    fn blank_keys_are_dropped() {
        let cfg = Config {
            api_keys: vec!["".into(), "  ".into(), "real".into()],
            ..Default::default()
        };
        assert_eq!(cfg.keys(), vec!["real".to_string()]);
    }

    #[test]
    fn split_keys_handles_commas_and_whitespace() {
        assert_eq!(split_keys("a,b , c"), vec!["a", "b", "c"]);
        assert_eq!(split_keys("a b\tc\nd"), vec!["a", "b", "c", "d"]);
        assert_eq!(split_keys("a;b,c"), vec!["a", "b", "c"]);
        assert!(split_keys("   ").is_empty());
    }

    #[test]
    fn dedup_keys_keeps_first_occurrence() {
        assert_eq!(
            dedup_keys(vec!["x".into(), "y".into(), "x".into(), "z".into()]),
            vec!["x", "y", "z"]
        );
    }
}
