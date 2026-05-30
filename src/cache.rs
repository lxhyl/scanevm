//! A tiny, dependency-free on-disk response cache.
//!
//! Each entry is one JSON file under `~/.scanevm/cache/`, named by a hash of
//! the cache key, holding `{ expires_at, data }`. This keeps the static musl
//! binary small (no embedded DB engine) while giving per-query TTLs:
//! verified source / ABI / bytecode are effectively immutable (cached
//! permanently), balances expire after seconds.
//!
//! Set `SCANEVM_NO_CACHE=1` to disable reads and writes entirely.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
struct Entry {
    /// Unix seconds after which the entry is stale. `None` = never expires.
    expires_at: Option<u64>,
    data: String,
}

pub struct Cache {
    dir: Option<PathBuf>,
    enabled: bool,
}

impl Cache {
    /// Build the default cache rooted at `~/.scanevm/cache`, honoring
    /// `SCANEVM_NO_CACHE`.
    pub fn new() -> Self {
        let disabled = std::env::var_os("SCANEVM_NO_CACHE")
            .map(|v| !v.is_empty())
            .unwrap_or(false);
        let dir = dirs::home_dir().map(|h| h.join(".scanevm").join("cache"));
        let enabled = !disabled && dir.is_some();
        Cache { dir, enabled }
    }

    /// Cache rooted at an explicit directory (used in tests).
    #[cfg(test)]
    pub fn with_dir(dir: PathBuf) -> Self {
        Cache {
            dir: Some(dir),
            enabled: true,
        }
    }

    /// Return cached `data` for `key` if present and not expired.
    pub fn get(&self, key: &str) -> Option<String> {
        if !self.enabled {
            return None;
        }
        let path = self.path(key)?;
        let raw = std::fs::read_to_string(&path).ok()?;
        let entry: Entry = serde_json::from_str(&raw).ok()?;
        if let Some(exp) = entry.expires_at {
            if now() >= exp {
                let _ = std::fs::remove_file(&path);
                return None;
            }
        }
        Some(entry.data)
    }

    /// Store `data` under `key` with an optional TTL (`None` = permanent).
    /// Failures are silently ignored — the cache is best-effort.
    pub fn put(&self, key: &str, data: &str, ttl: Option<Duration>) {
        if !self.enabled {
            return;
        }
        let Some(path) = self.path(key) else {
            return;
        };
        if let Some(parent) = path.parent() {
            if std::fs::create_dir_all(parent).is_err() {
                return;
            }
        }
        let entry = Entry {
            expires_at: ttl.map(|d| now().saturating_add(d.as_secs())),
            data: data.to_string(),
        };
        let Ok(serialized) = serde_json::to_string(&entry) else {
            return;
        };
        // Write to a temp file then rename so readers never see a partial file.
        let tmp = path.with_extension("tmp");
        if std::fs::write(&tmp, serialized.as_bytes()).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }

    fn path(&self, key: &str) -> Option<PathBuf> {
        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        Some(
            self.dir
                .as_ref()?
                .join(format!("{:016x}.json", hasher.finish())),
        )
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("scanevm-cache-test-{}-{}", std::process::id(), tag))
    }

    #[test]
    fn put_then_get_roundtrips() {
        let dir = temp_dir("roundtrip");
        let _ = std::fs::remove_dir_all(&dir);
        let cache = Cache::with_dir(dir.clone());
        cache.put("k1", "hello", None);
        assert_eq!(cache.get("k1").as_deref(), Some("hello"));
        assert_eq!(cache.get("missing"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn expired_entry_is_ignored() {
        let dir = temp_dir("expiry");
        let _ = std::fs::remove_dir_all(&dir);
        let cache = Cache::with_dir(dir.clone());
        cache.put("k", "stale", Some(Duration::from_secs(0)));
        // TTL of 0 seconds means expires_at == now, so it must read back as a miss.
        assert_eq!(cache.get("k"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn distinct_keys_do_not_collide() {
        let dir = temp_dir("keys");
        let _ = std::fs::remove_dir_all(&dir);
        let cache = Cache::with_dir(dir.clone());
        cache.put("a", "1", None);
        cache.put("b", "2", None);
        assert_eq!(cache.get("a").as_deref(), Some("1"));
        assert_eq!(cache.get("b").as_deref(), Some("2"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
