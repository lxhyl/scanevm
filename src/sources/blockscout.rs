//! Blockscout API v2: `GET {instance}/api/v2/smart-contracts/{address}`.

use serde_json::Value;

use super::{
    absorb_settings, lang_ext, language_loose, non_empty, norm_hex, Compiler, Fetched, Http,
    Optimizer, Outcome, ProxyHint, Quality, Target,
};
use crate::chains::{ChainInfo, Provider};
use crate::error::{Result, ScanevmError};

pub async fn fetch(http: &Http, chain: &ChainInfo, address: &str) -> Result<Outcome> {
    let Some(base) = chain.blockscout else {
        return Ok(Outcome::Skipped(format!(
            "no Blockscout instance for {}",
            chain.label()
        )));
    };
    let url = format!("{base}/api/v2/smart-contracts/{address}");
    let (status, body) = http.get(&url, "blockscout").await?;
    if status == 404 {
        return Ok(Outcome::NotVerified);
    }
    if !(200..300).contains(&status) {
        return Err(ScanevmError::ApiError(format!("blockscout: HTTP {status}")));
    }
    let v: Value = serde_json::from_str(&body)
        .map_err(|_| ScanevmError::ApiError(format!("blockscout: HTTP {status}, not JSON")))?;
    Ok(match parse(&v, url) {
        Some(f) => Outcome::Found(Box::new(f)),
        None => Outcome::NotVerified,
    })
}

pub fn parse(v: &Value, url: String) -> Option<Fetched> {
    let main_src = v["source_code"].as_str().filter(|s| !s.trim().is_empty())?;
    let verified = v["is_verified"].as_bool() == Some(true);
    let twin = non_empty(v, "verified_twin_address_hash");
    let full = v["is_fully_verified"].as_bool() == Some(true);
    let partial = v["is_partially_verified"].as_bool() == Some(true);
    let (quality, similar_to) = if !verified {
        (Quality::Similar, Some(twin?))
    } else if partial && !full {
        (Quality::Partial, None)
    } else {
        (Quality::Verified, None)
    };

    let name = non_empty(v, "name").unwrap_or_default();
    let language = match non_empty(v, "language") {
        Some(l) => language_loose(&l),
        None if v["is_vyper_contract"].as_bool() == Some(true) => "vyper".into(),
        None => "solidity".into(),
    };
    let mut compiler = Compiler {
        version: non_empty(v, "compiler_version")
            .unwrap_or_default()
            .trim_start_matches("vyper:")
            .trim_start_matches('v')
            .to_string(),
        ..Default::default()
    };
    if let Some(enabled) = v["optimization_enabled"].as_bool() {
        compiler.optimizer = Some(Optimizer {
            enabled,
            runs: v["optimization_runs"].as_u64(),
        });
    }
    if let Some(evm) = non_empty(v, "evm_version").filter(|e| !e.eq_ignore_ascii_case("default")) {
        compiler.evm_version = Some(evm);
    }
    if let Some(s) = v.get("compiler_settings").filter(|s| s.is_object()) {
        absorb_settings(&mut compiler, s);
    }
    if compiler.libraries.is_none() {
        let libs: serde_json::Map<String, Value> = v["external_libraries"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|l| {
                        Some((
                            non_empty(l, "name")?,
                            Value::String(non_empty(l, "address_hash")?),
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default();
        if !libs.is_empty() {
            compiler.libraries = Some(Value::Object(libs));
        }
    }

    let main_path = non_empty(v, "file_path").unwrap_or_else(|| {
        format!(
            "{}.{}",
            if name.is_empty() { "Contract" } else { &name },
            lang_ext(&language)
        )
    });
    let mut sources = vec![(main_path.clone(), main_src.to_string())];
    for f in v["additional_sources"].as_array().into_iter().flatten() {
        if let (Some(p), Some(c)) = (non_empty(f, "file_path"), f["source_code"].as_str()) {
            if !sources.iter().any(|(q, _)| *q == p) {
                sources.push((p, c.to_string()));
            }
        }
    }

    let implementations: Vec<String> = v["implementations"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|i| non_empty(i, "address_hash").or_else(|| non_empty(i, "address")))
                .collect()
        })
        .unwrap_or_default();
    let kind = non_empty(v, "proxy_type");
    let proxy_hint = (kind.is_some() || !implementations.is_empty()).then_some(ProxyHint {
        is_proxy: true,
        kind,
        implementations,
    });

    Some(Fetched {
        provider: Provider::Blockscout,
        quality,
        similar_to,
        target: (!name.is_empty()).then(|| Target {
            path: main_path,
            name: name.clone(),
        }),
        name,
        language,
        compiler,
        sources,
        abi: v.get("abi").filter(|a| a.is_array()).cloned(),
        constructor_args: norm_hex(&non_empty(v, "constructor_args").unwrap_or_default()),
        proxy_hint,
        deployment: None,
        license: non_empty(v, "license_type").filter(|l| l != "none"),
        url,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_verified_contract() {
        let v = json!({
            "is_verified": true, "is_fully_verified": true, "name": "Foo",
            "language": "solidity", "compiler_version": "v0.8.19+commit.7dd6d404",
            "optimization_enabled": true, "optimization_runs": 999, "evm_version": "default",
            "file_path": "src/Foo.sol", "source_code": "contract Foo {}",
            "additional_sources": [{"file_path": "lib/L.sol", "source_code": "library L {}"}],
            "abi": [], "license_type": "mit", "constructor_args": "0x01",
            "proxy_type": "eip1967", "implementations": [{"address_hash": "0xaa", "name": "Impl"}]
        });
        let f = parse(&v, "u".into()).unwrap();
        assert_eq!(f.quality, Quality::Verified);
        assert_eq!(f.compiler.version, "0.8.19+commit.7dd6d404");
        assert!(f.compiler.evm_version.is_none());
        assert_eq!(f.sources.len(), 2);
        assert_eq!(f.target.unwrap().path, "src/Foo.sol");
        let hint = f.proxy_hint.unwrap();
        assert_eq!(
            (hint.kind.as_deref(), hint.implementations.len()),
            (Some("eip1967"), 1)
        );
    }

    #[test]
    fn twins_and_unverified() {
        let twin =
            json!({"is_verified": false, "verified_twin_address_hash": "0xbb", "source_code": "x"});
        let f = parse(&twin, "u".into()).unwrap();
        assert_eq!(
            (f.quality, f.similar_to.as_deref()),
            (Quality::Similar, Some("0xbb"))
        );
        assert!(parse(
            &json!({"is_verified": false, "source_code": "x"}),
            "u".into()
        )
        .is_none());
        assert!(parse(&json!({"is_verified": true}), "u".into()).is_none());
    }
}
