//! Etherscan v2 and Routescan (Etherscan-compatible) `getsourcecode`.
//!
//! `SourceCode` comes in three shapes, all unpacked here: a single file, a
//! JSON object `{ "<path>": { "content": … } }`, or `{{ … }}`-wrapped
//! standard-JSON input.

use serde_json::Value;

use super::{
    absorb_settings, guess_target, lang_ext, non_empty, norm_hex, Compiler, Fetched, Http,
    Optimizer, Outcome, ProxyHint, Quality,
};
use crate::chains::{ChainInfo, Provider};
use crate::client::EtherscanClient;
use crate::error::{Result, ScanevmError};

const ETHERSCAN_V2: &str = "https://api.etherscan.io/v2/api";

/// Etherscan, through the user's key pool (rate limiting and key rotation
/// live in [`EtherscanClient`]).
pub async fn fetch_etherscan(
    client: Option<&EtherscanClient>,
    chain: &ChainInfo,
    address: &str,
) -> Result<Outcome> {
    let Some(client) = client else {
        return Ok(Outcome::Skipped(
            "no Etherscan API key (scanevm config set-key <KEY>)".to_string(),
        ));
    };
    let url = format!(
        "{ETHERSCAN_V2}?chainid={}&module=contract&action=getsourcecode&address={address}",
        chain.chain_id
    );
    let result: Value = match client
        .call(&[
            ("module", "contract"),
            ("action", "getsourcecode"),
            ("address", address),
        ])
        .await
    {
        Ok(v) => v,
        Err(ScanevmError::ApiError(m)) if is_unsupported(&m) => {
            return Ok(Outcome::Unsupported(format!("etherscan: {m}")))
        }
        Err(e) => return Err(e),
    };
    finish(&result, Provider::Etherscan, url)
}

/// Routescan's keyless Etherscan-compatible API.
pub async fn fetch_routescan(http: &Http, chain: &ChainInfo, address: &str) -> Result<Outcome> {
    let net = chain.routescan.unwrap_or(if chain.is_testnet {
        "testnet"
    } else {
        "mainnet"
    });
    let url = format!(
        "https://api.routescan.io/v2/network/{net}/evm/{}/etherscan/api?module=contract&action=getsourcecode&address={address}",
        chain.chain_id
    );
    let (status, body) = http.get(&url, "routescan").await?;
    let v: Value = serde_json::from_str(&body)
        .map_err(|_| ScanevmError::ApiError(format!("routescan: HTTP {status}, not JSON")))?;
    if v["status"].as_str() == Some("1") {
        return finish(&v["result"], Provider::Routescan, url);
    }
    let message = v["message"].as_str().unwrap_or("");
    let result = v["result"].as_str().unwrap_or("");
    let text = if result.is_empty() {
        message.to_string()
    } else {
        format!("{message}: {result}")
    };
    let lower = text.to_lowercase();
    if is_unsupported(&text) {
        Ok(Outcome::Unsupported(format!("routescan: {text}")))
    } else if lower.contains("rate limit") || lower.contains("too many") {
        Err(ScanevmError::RateLimited { retry_after: None })
    } else {
        Err(ScanevmError::ApiError(format!("routescan: {text}")))
    }
}

/// "This chain isn't served (on this plan)": ask the next provider.
fn is_unsupported(msg: &str) -> bool {
    let m = msg.to_lowercase();
    m.contains("not supported")
        || m.contains("invalid chainid")
        || m.contains("unsupported chain")
        || m.contains("missing or unsupported chainid")
        || m.contains("free api access")
        || m.contains("upgrade your api plan")
}

fn finish(result: &Value, provider: Provider, url: String) -> Result<Outcome> {
    let item = match result {
        Value::Array(a) if !a.is_empty() => &a[0],
        Value::Object(_) => result,
        // An empty answer for an address is "nothing verified here".
        Value::Array(_) | Value::Null => return Ok(Outcome::NotVerified),
        other => {
            return Err(ScanevmError::ApiError(format!(
                "{}: unexpected result {}",
                provider.as_str(),
                other.to_string().chars().take(120).collect::<String>()
            )))
        }
    };
    Ok(match parse_item(item, provider, url)? {
        Some(f) => Outcome::Found(Box::new(f)),
        None => Outcome::NotVerified,
    })
}

/// Parse one `getsourcecode` item. `None` = not verified.
pub fn parse_item(item: &Value, provider: Provider, url: String) -> Result<Option<Fetched>> {
    let source_code = item["SourceCode"].as_str().unwrap_or("");
    let abi_raw = non_empty(item, "ABI").unwrap_or_default();
    if source_code.trim().is_empty()
        || abi_raw.eq_ignore_ascii_case("contract source code not verified")
    {
        return Ok(None);
    }
    let name = non_empty(item, "ContractName").unwrap_or_default();
    let version = non_empty(item, "CompilerVersion").unwrap_or_default();
    let ctype = non_empty(item, "CompilerType").unwrap_or_default();
    let language =
        if version.to_lowercase().starts_with("vyper") || ctype.eq_ignore_ascii_case("vyper") {
            "vyper"
        } else {
            "solidity"
        }
        .to_string();

    let mut compiler = Compiler {
        version: version
            .trim_start_matches("vyper:")
            .trim_start_matches('v')
            .to_string(),
        ..Default::default()
    };
    if let Some(opt) = non_empty(item, "OptimizationUsed") {
        // Vyper has no optimizer runs; Etherscan fills the field anyway.
        let runs = non_empty(item, "Runs")
            .filter(|_| language != "vyper")
            .and_then(|r| r.parse().ok());
        compiler.optimizer = Some(Optimizer {
            enabled: opt == "1",
            runs,
        });
    }
    if let Some(evm) = non_empty(item, "EVMVersion").filter(|e| !e.eq_ignore_ascii_case("default"))
    {
        compiler.evm_version = Some(evm);
    }
    let libs = parse_library_field(&non_empty(item, "Library").unwrap_or_default());
    if !libs.is_empty() {
        compiler.libraries = Some(Value::Object(libs));
    }

    let (sources, settings) = unpack_source_code(source_code, &name, &language)?;
    if let Some(s) = settings {
        absorb_settings(&mut compiler, &s);
    }
    let target = guess_target(&sources, &name, &language);

    let (quality, similar_to) = match non_empty(item, "SimilarMatch") {
        Some(s) => (Quality::Similar, Some(s)),
        None => (Quality::Verified, None),
    };
    // Etherscan sometimes fills Implementation for non-proxies (e.g. with a
    // linked library), so only the Proxy flag counts.
    let proxy_hint = (non_empty(item, "Proxy").as_deref() == Some("1")).then(|| ProxyHint {
        is_proxy: true,
        kind: None,
        implementations: non_empty(item, "Implementation").into_iter().collect(),
    });
    let license = non_empty(item, "LicenseType").filter(|l| {
        let l = l.to_lowercase();
        l != "unknown" && l != "none"
    });
    let abi = serde_json::from_str::<Value>(&abi_raw)
        .ok()
        .filter(Value::is_array);

    Ok(Some(Fetched {
        provider,
        quality,
        similar_to,
        name,
        language,
        compiler,
        target,
        sources,
        abi,
        constructor_args: norm_hex(&non_empty(item, "ConstructorArguments").unwrap_or_default()),
        proxy_hint,
        deployment: None,
        license,
        url,
    }))
}

/// `Lib1:0xaddr;Lib2:0xaddr` → `{ "Lib1": "0x…" }`.
fn parse_library_field(s: &str) -> serde_json::Map<String, Value> {
    let mut m = serde_json::Map::new();
    for part in s.split(';').map(str::trim).filter(|p| !p.is_empty()) {
        if let Some((k, v)) = part.split_once(':') {
            let v = v.trim();
            let v = if v.starts_with("0x") {
                v.to_string()
            } else {
                format!("0x{v}")
            };
            m.insert(k.trim().to_string(), Value::String(v));
        }
    }
    m
}

type Unpacked = (Vec<(String, String)>, Option<Value>);

/// Split `SourceCode` into (path, content) files, plus the standard-JSON
/// `settings` when there were any.
pub fn unpack_source_code(code: &str, name: &str, lang: &str) -> Result<Unpacked> {
    let t = code.trim();
    // Shape 3: {{ standard json }}
    if t.starts_with("{{") && t.ends_with("}}") {
        let v: Value = serde_json::from_str(&t[1..t.len() - 1])
            .map_err(|e| ScanevmError::ApiError(format!("bad standard-JSON SourceCode: {e}")))?;
        return Ok((sources_from_map(&v["sources"])?, v.get("settings").cloned()));
    }
    // Shape 2: { "path": { "content": … } } (or standard JSON without the double braces)
    if t.starts_with('{') {
        if let Ok(v) = serde_json::from_str::<Value>(t) {
            if v["sources"].is_object() {
                return Ok((sources_from_map(&v["sources"])?, v.get("settings").cloned()));
            }
            if let Some(m) = v.as_object() {
                if !m.is_empty() && m.values().all(|x| x.get("content").is_some()) {
                    return Ok((sources_from_map(&v)?, None));
                }
            }
        }
    }
    // Shape 1: one flattened file.
    let file = format!(
        "{}.{}",
        if name.is_empty() { "Contract" } else { name },
        lang_ext(lang)
    );
    Ok((vec![(file, code.to_string())], None))
}

fn sources_from_map(v: &Value) -> Result<Vec<(String, String)>> {
    let m = v
        .as_object()
        .ok_or_else(|| ScanevmError::ApiError("standard-JSON SourceCode without sources".into()))?;
    let out: Vec<(String, String)> = m
        .iter()
        .map(|(p, f)| (p.clone(), f["content"].as_str().unwrap_or("").to_string()))
        .collect();
    if out.is_empty() {
        return Err(ScanevmError::ApiError(
            "SourceCode has no source files".into(),
        ));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn item(source: &str) -> Value {
        json!({
            "SourceCode": source, "ABI": "[{\"type\":\"function\",\"name\":\"f\"}]",
            "ContractName": "Foo", "CompilerVersion": "v0.8.20+commit.a1b79de6",
            "OptimizationUsed": "1", "Runs": "200", "EVMVersion": "Default",
            "Library": "", "LicenseType": "MIT", "Proxy": "1",
            "Implementation": "0x00000000000000000000000000000000000000aa",
            "ConstructorArguments": "ABCD", "SimilarMatch": ""
        })
    }

    #[test]
    fn single_file_item() {
        let f = parse_item(&item("contract Foo {}"), Provider::Etherscan, "u".into())
            .unwrap()
            .unwrap();
        assert_eq!(f.name, "Foo");
        assert_eq!(f.quality, Quality::Verified);
        assert_eq!(f.compiler.version, "0.8.20+commit.a1b79de6");
        assert_eq!(
            f.compiler.optimizer,
            Some(Optimizer {
                enabled: true,
                runs: Some(200)
            })
        );
        assert!(f.compiler.evm_version.is_none());
        assert_eq!(
            f.sources,
            [("Foo.sol".to_string(), "contract Foo {}".to_string())]
        );
        assert_eq!(f.target.unwrap().path, "Foo.sol");
        assert!(f.abi.unwrap().is_array());
        assert_eq!(f.constructor_args.as_deref(), Some("0xabcd"));
        assert_eq!(f.proxy_hint.unwrap().implementations.len(), 1);
        assert_eq!(f.license.as_deref(), Some("MIT"));
    }

    #[test]
    fn standard_json_item() {
        let src = r#"{{"language":"Solidity","sources":{"a/Foo.sol":{"content":"contract Foo {}"},"b/L.sol":{"content":"library L {}"}},"settings":{"optimizer":{"enabled":false},"evmVersion":"shanghai"}}}"#;
        let f = parse_item(&item(src), Provider::Etherscan, "u".into())
            .unwrap()
            .unwrap();
        assert_eq!(f.sources.len(), 2);
        assert_eq!(f.target.unwrap().path, "a/Foo.sol");
        assert_eq!(f.compiler.evm_version.as_deref(), Some("shanghai"));
        assert_eq!(
            f.compiler.optimizer,
            Some(Optimizer {
                enabled: false,
                runs: None
            })
        );
    }

    #[test]
    fn bare_map_and_unverified() {
        let (files, settings) =
            unpack_source_code(r#"{"Only.sol":{"content":"Z"}}"#, "Only", "solidity").unwrap();
        assert_eq!(files, [("Only.sol".to_string(), "Z".to_string())]);
        assert!(settings.is_none());
        let unverified = json!({"SourceCode": "", "ABI": "Contract source code not verified"});
        assert!(parse_item(&unverified, Provider::Etherscan, "u".into())
            .unwrap()
            .is_none());
        assert!(matches!(
            finish(&json!([]), Provider::Routescan, "u".into()),
            Ok(Outcome::NotVerified)
        ));
    }

    #[test]
    fn unsupported_chain_messages() {
        assert!(is_unsupported("Missing or unsupported chainid parameter"));
        assert!(is_unsupported("chain not supported"));
        assert!(!is_unsupported("Max rate limit reached"));
    }

    #[test]
    fn library_field() {
        let m = parse_library_field("A:0x01; B:02");
        assert_eq!(m["A"], "0x01");
        assert_eq!(m["B"], "0x02");
    }
}
