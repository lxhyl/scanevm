//! Sourcify v2: `GET {server}/v2/contract/{chainId}/{address}?fields=…`.

use serde_json::Value;

use super::{
    absorb_settings, as_str, language_loose, norm_hex, split_fqn, Compiler, Deployment, Fetched,
    Http, Outcome, ProxyHint, Quality, Target,
};
use crate::chains::{ChainInfo, Provider};
use crate::error::{Result, ScanevmError};

const SERVER: &str = "https://sourcify.dev/server";
const FIELDS: &str = "abi,sources,compilation,proxyResolution,deployment";

pub async fn fetch(http: &Http, chain: &ChainInfo, address: &str) -> Result<Outcome> {
    let url = format!(
        "{SERVER}/v2/contract/{}/{address}?fields={FIELDS}",
        chain.chain_id
    );
    let (status, body) = http.get(&url, "sourcify").await?;
    if status == 404 {
        return Ok(Outcome::NotVerified);
    }
    let v: Value = serde_json::from_str(&body)
        .map_err(|_| ScanevmError::ApiError(format!("sourcify: HTTP {status}, not JSON")))?;
    if status == 400 {
        let msg = as_str(&v, "message").unwrap_or_else(|| "bad request".into());
        if msg.to_lowercase().contains("chain") {
            return Ok(Outcome::Unsupported(format!("sourcify: {msg}")));
        }
        return Err(ScanevmError::ApiError(format!("sourcify: {msg}")));
    }
    if !(200..300).contains(&status) {
        return Err(ScanevmError::ApiError(format!("sourcify: HTTP {status}")));
    }
    Ok(match parse(&v, url)? {
        Some(f) => Outcome::Found(Box::new(f)),
        None => Outcome::NotVerified,
    })
}

pub fn parse(v: &Value, url: String) -> Result<Option<Fetched>> {
    let quality = match v["match"].as_str() {
        Some("exact_match") => Quality::Exact,
        Some("match") => Quality::Partial,
        _ => return Ok(None),
    };
    let comp = &v["compilation"];
    let language = language_loose(comp["language"].as_str().unwrap_or("solidity"));
    let mut compiler = Compiler {
        version: comp["compilerVersion"]
            .as_str()
            .unwrap_or("")
            .trim_start_matches('v')
            .to_string(),
        ..Default::default()
    };
    if let Some(s) = comp.get("compilerSettings") {
        absorb_settings(&mut compiler, s);
    }
    let target = comp["fullyQualifiedName"]
        .as_str()
        .and_then(split_fqn)
        .map(|(path, name)| Target { path, name });
    let name = as_str(comp, "name")
        .or_else(|| target.as_ref().map(|t| t.name.clone()))
        .unwrap_or_default();

    let sources: Vec<(String, String)> = v["sources"]
        .as_object()
        .map(|m| {
            m.iter()
                .map(|(p, f)| (p.clone(), f["content"].as_str().unwrap_or("").to_string()))
                .collect()
        })
        .unwrap_or_default();
    if sources.is_empty() {
        return Err(ScanevmError::ApiError(
            "sourcify: verified but returned no sources".into(),
        ));
    }

    let pr = &v["proxyResolution"];
    let proxy_hint = pr.is_object().then(|| ProxyHint {
        is_proxy: pr["isProxy"].as_bool() == Some(true),
        kind: as_str(pr, "proxyType"),
        implementations: pr["implementations"]
            .as_array()
            .map(|a| a.iter().filter_map(|i| as_str(i, "address")).collect())
            .unwrap_or_default(),
    });
    let d = &v["deployment"];
    let deployment = d.is_object().then(|| Deployment {
        tx_hash: as_str(d, "transactionHash"),
        block: d["blockNumber"]
            .as_u64()
            .or_else(|| d["blockNumber"].as_str().and_then(|s| s.parse().ok())),
        deployer: as_str(d, "deployer"),
    });
    let constructor_args = d["constructorArguments"].as_str().and_then(norm_hex);

    Ok(Some(Fetched {
        provider: Provider::Sourcify,
        quality,
        similar_to: None,
        name,
        language,
        compiler,
        target,
        sources,
        abi: v.get("abi").filter(|a| a.is_array()).cloned(),
        constructor_args,
        proxy_hint,
        deployment,
        license: None,
        url,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_exact_match() {
        let v = json!({
            "match": "exact_match",
            "abi": [{"type": "function", "name": "f"}],
            "compilation": {"language": "Solidity", "compilerVersion": "0.8.12+commit.f00d7308",
                "name": "Multicall3", "fullyQualifiedName": "src/Multicall3.sol:Multicall3",
                "compilerSettings": {"optimizer": {"enabled": true, "runs": 10000000}}},
            "sources": {"src/Multicall3.sol": {"content": "contract Multicall3 {}"}},
            "proxyResolution": {"isProxy": false, "proxyType": null, "implementations": []},
            "deployment": {"transactionHash": "0xab", "blockNumber": "14353601", "deployer": "0xcd"}
        });
        let f = parse(&v, "u".into()).unwrap().unwrap();
        assert_eq!(f.quality, Quality::Exact);
        assert_eq!(f.name, "Multicall3");
        assert_eq!(f.target.unwrap().path, "src/Multicall3.sol");
        assert_eq!(f.compiler.optimizer.unwrap().runs, Some(10_000_000));
        assert!(!f.proxy_hint.unwrap().is_proxy);
        assert_eq!(f.deployment.unwrap().block, Some(14_353_601));
        assert!(f.abi.is_some());
    }

    #[test]
    fn partial_and_unverified() {
        let v =
            json!({"match": "match", "compilation": {}, "sources": {"A.sol": {"content": "x"}}});
        assert_eq!(
            parse(&v, "u".into()).unwrap().unwrap().quality,
            Quality::Partial
        );
        assert!(parse(&json!({"match": null}), "u".into())
            .unwrap()
            .is_none());
        let empty = json!({"match": "match", "sources": {}});
        assert!(parse(&empty, "u".into()).is_err());
    }
}
