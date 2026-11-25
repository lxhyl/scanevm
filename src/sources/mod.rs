//! Verified-source providers. Each turns its API into a [`Fetched`]: the same
//! normalised shape whatever the provider, so the rest of scanevm (and
//! `--json`) doesn't care where the source came from.
//!
//! Ported from the scan core (core/src/providers, corejs/src/providers):
//! Sourcify v2, Etherscan v2, Blockscout v2 and Routescan (Etherscan-compatible).

pub mod blockscout;
pub mod etherscan;
pub mod sourcify;

use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::time::sleep;

use crate::chains::Provider;
use crate::client::{backoff_delay, retry_after_secs};
use crate::error::{Result, ScanevmError};

/// How good a provider's match is. `similar` is a twin contract's source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Quality {
    Exact,
    Verified,
    Partial,
    Similar,
}

impl Quality {
    pub fn as_str(self) -> &'static str {
        match self {
            Quality::Exact => "exact",
            Quality::Verified => "verified",
            Quality::Partial => "partial",
            Quality::Similar => "similar",
        }
    }
}

/// Ranking: Sourcify exact > Etherscan > Sourcify partial > Blockscout > Routescan.
pub fn rank(p: Provider, q: Quality) -> u32 {
    if q == Quality::Similar {
        return 5;
    }
    match p {
        Provider::Sourcify if q == Quality::Exact => 100,
        Provider::Sourcify => 60,
        Provider::Etherscan => 80,
        Provider::Blockscout if q == Quality::Partial => 40,
        Provider::Blockscout => 50,
        Provider::Routescan => 45,
    }
}

/// The best rank a provider can return; lets the fetch stop early.
pub fn best_possible_rank(p: Provider) -> u32 {
    match p {
        Provider::Sourcify => 100,
        Provider::Etherscan => 80,
        Provider::Blockscout => 50,
        Provider::Routescan => 45,
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Optimizer {
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runs: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Compiler {
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub optimizer: Option<Optimizer>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evm_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via_ir: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remappings: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub libraries: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Target {
    pub path: String,
    pub name: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProxyHint {
    pub is_proxy: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub implementations: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Deployment {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tx_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deployer: Option<String>,
}

/// One provider's verified source, normalised.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fetched {
    pub provider: Provider,
    pub quality: Quality,
    pub similar_to: Option<String>,
    pub name: String,
    /// solidity | vyper | yul | unknown
    pub language: String,
    pub compiler: Compiler,
    pub target: Option<Target>,
    /// (original path, content), in provider order.
    pub sources: Vec<(String, String)>,
    /// The ABI (a JSON array), when the provider gave one.
    pub abi: Option<Value>,
    pub constructor_args: Option<String>,
    pub proxy_hint: Option<ProxyHint>,
    pub deployment: Option<Deployment>,
    pub license: Option<String>,
    /// Public request URL (never carries a key).
    pub url: String,
}

impl Fetched {
    pub fn rank(&self) -> u32 {
        rank(self.provider, self.quality)
    }
}

/// What asking one provider came to (errors are the `Err` side).
pub enum Outcome {
    Found(Box<Fetched>),
    /// The provider answered: no verified source at this address.
    NotVerified,
    /// The provider doesn't serve this chain.
    Unsupported(String),
    /// Not asked (no key, no instance for this chain).
    Skipped(String),
}

// ---- HTTP for the keyless providers ----

pub struct Http {
    client: reqwest::Client,
}

/// Retries for transient trouble (timeouts, 429, 5xx) per provider request.
const RETRIES: u32 = 2;

impl Http {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .user_agent(concat!("scanevm/", env!("CARGO_PKG_VERSION")))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self { client }
    }

    /// GET `url`, retrying transient failures; returns (status, body) for any
    /// answer that isn't a 429 or 5xx.
    pub async fn get(&self, url: &str, who: &str) -> Result<(u16, String)> {
        let mut attempt = 0;
        loop {
            match self.client.get(url).send().await {
                Ok(resp) => {
                    let status = resp.status().as_u16();
                    let retry_after = retry_after_secs(&resp);
                    if status == 429 || status >= 500 {
                        if attempt < RETRIES {
                            attempt += 1;
                            sleep(backoff_delay(attempt, retry_after.map(|s| s.min(5)))).await;
                            continue;
                        }
                        return Err(if status == 429 {
                            ScanevmError::RateLimited { retry_after }
                        } else {
                            ScanevmError::Network(format!("{who}: HTTP {status}"))
                        });
                    }
                    let body = resp.text().await.map_err(ScanevmError::from_http)?;
                    return Ok((status, body));
                }
                Err(e) => {
                    if (e.is_timeout() || e.is_connect()) && attempt < RETRIES {
                        attempt += 1;
                        sleep(backoff_delay(attempt, None)).await;
                        continue;
                    }
                    let e = ScanevmError::from_http(e);
                    return Err(ScanevmError::Network(format!("{who}: {e}")));
                }
            }
        }
    }
}

// ---- shared parsing ----

pub fn as_str(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(str::to_string)
}

/// A trimmed, non-empty string field.
pub fn non_empty(v: &Value, k: &str) -> Option<String> {
    v.get(k)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// `0x` + lowercase hex, or `None` for empty input.
pub fn norm_hex(s: &str) -> Option<String> {
    let t = s.trim().trim_start_matches("0x");
    (!t.is_empty()).then(|| format!("0x{}", t.to_ascii_lowercase()))
}

pub fn language_loose(s: &str) -> String {
    let l = s.to_lowercase();
    if l.contains("vyper") {
        "vyper"
    } else if l.contains("yul") {
        "yul"
    } else if l.contains("solidity") || l.contains("solc") {
        "solidity"
    } else {
        "unknown"
    }
    .to_string()
}

pub fn lang_ext(lang: &str) -> &'static str {
    match lang {
        "vyper" => "vy",
        "yul" => "yul",
        _ => "sol",
    }
}

/// Fill optimizer / evm version / viaIR / remappings / libraries from a
/// standard-JSON `settings` object.
pub fn absorb_settings(c: &mut Compiler, settings: &Value) {
    let Some(s) = settings.as_object() else {
        return;
    };
    if let Some(o) = s.get("optimizer").and_then(Value::as_object) {
        let enabled = o.get("enabled").and_then(Value::as_bool);
        let runs = o.get("runs").and_then(Value::as_u64);
        if enabled.is_some() || runs.is_some() {
            c.optimizer = Some(Optimizer {
                enabled: enabled.unwrap_or(false),
                runs,
            });
        }
    }
    if let Some(e) = s.get("evmVersion").and_then(Value::as_str) {
        c.evm_version = Some(e.to_string());
    }
    if let Some(v) = s.get("viaIR").and_then(Value::as_bool) {
        c.via_ir = Some(v);
    }
    if let Some(r) = s.get("remappings").and_then(Value::as_array) {
        let r: Vec<String> = r
            .iter()
            .filter_map(|x| x.as_str().map(str::to_string))
            .collect();
        if !r.is_empty() {
            c.remappings = r;
        }
    }
    if let Some(l) = s.get("libraries") {
        if l.as_object().is_some_and(|m| !m.is_empty()) {
            c.libraries = Some(l.clone());
        }
    }
}

/// `path/to/A.sol:A` → (path, name).
pub fn split_fqn(fqn: &str) -> Option<(String, String)> {
    let (p, n) = fqn.rsplit_once(':')?;
    (!p.is_empty() && !n.is_empty()).then(|| (p.to_string(), n.to_string()))
}

fn stem(path: &str) -> Option<&str> {
    let file = path.rsplit('/').next().unwrap_or(path);
    file.rsplit_once('.').map(|(s, _)| s)
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The file that declares `contract <name>` (Etherscan doesn't say which one).
pub fn guess_target(sources: &[(String, String)], name: &str, lang: &str) -> Option<Target> {
    if name.is_empty() {
        return None;
    }
    let hit = |path: &str| {
        Some(Target {
            path: path.to_string(),
            name: name.to_string(),
        })
    };
    if sources.len() == 1 {
        return hit(&sources[0].0);
    }
    if lang == "vyper" {
        return sources
            .iter()
            .find(|(p, _)| stem(p) == Some(name))
            .and_then(|(p, _)| hit(p));
    }
    let needles = [
        format!("contract {name}"),
        format!("library {name}"),
        format!("interface {name}"),
    ];
    let declares = |code: &str| {
        needles.iter().any(|n| {
            code.match_indices(n.as_str()).any(|(i, _)| {
                let after = code[i + n.len()..].chars().next();
                let before = code[..i].chars().next_back();
                after.is_some_and(|c| !is_word(c)) && before.is_none_or(|c| !is_word(c))
            })
        })
    };
    sources
        .iter()
        .find(|(p, c)| stem(p) == Some(name) && declares(c))
        .or_else(|| sources.iter().find(|(_, c)| declares(c)))
        .and_then(|(p, _)| hit(p))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn ranking_prefers_exact_then_etherscan() {
        assert!(
            rank(Provider::Sourcify, Quality::Exact) > rank(Provider::Etherscan, Quality::Verified)
        );
        assert!(
            rank(Provider::Etherscan, Quality::Verified)
                > rank(Provider::Sourcify, Quality::Partial)
        );
        assert!(
            rank(Provider::Blockscout, Quality::Verified)
                > rank(Provider::Routescan, Quality::Verified)
        );
        assert_eq!(rank(Provider::Etherscan, Quality::Similar), 5);
    }

    #[test]
    fn settings_are_absorbed() {
        let mut c = Compiler::default();
        absorb_settings(
            &mut c,
            &json!({"optimizer": {"enabled": true, "runs": 200}, "evmVersion": "paris",
                    "viaIR": true, "remappings": ["a=b"], "libraries": {}}),
        );
        assert_eq!(
            c.optimizer,
            Some(Optimizer {
                enabled: true,
                runs: Some(200)
            })
        );
        assert_eq!(c.evm_version.as_deref(), Some("paris"));
        assert_eq!(c.via_ir, Some(true));
        assert_eq!(c.remappings, ["a=b"]);
        assert!(c.libraries.is_none());
    }

    #[test]
    fn target_is_the_declaring_file() {
        let src = vec![
            ("lib/IFoo.sol".to_string(), "interface IFoo {}".to_string()),
            (
                "src/Foo.sol".to_string(),
                "contract FooBar {} contract Foo is IFoo {}".to_string(),
            ),
        ];
        assert_eq!(
            guess_target(&src, "Foo", "solidity").unwrap().path,
            "src/Foo.sol"
        );
        assert!(guess_target(&src, "Fo", "solidity").is_none());
        assert_eq!(
            guess_target(&src, "IFoo", "solidity").unwrap().path,
            "lib/IFoo.sol"
        );
    }

    #[test]
    fn small_parsers() {
        assert_eq!(norm_hex("0xABcd").as_deref(), Some("0xabcd"));
        assert_eq!(norm_hex(""), None);
        assert_eq!(split_fqn("a/B.sol:B"), Some(("a/B.sol".into(), "B".into())));
        assert_eq!(split_fqn(":B"), None);
        assert_eq!(language_loose("Vyper"), "vyper");
        assert_eq!(language_loose("Solidity"), "solidity");
    }
}
