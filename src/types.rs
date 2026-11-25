use crate::client::CachePolicy;
use serde::{Deserialize, Deserializer, Serialize};

fn empty_string_as_none<'de, D>(d: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let s = Option::<String>::deserialize(d)?;
    Ok(s.filter(|v| !v.is_empty()))
}

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct ContractSource {
    #[serde(rename = "SourceCode")]
    pub source_code: String,
    #[serde(rename = "ABI")]
    pub abi: String,
    #[serde(rename = "ContractName")]
    pub contract_name: String,
    #[serde(rename = "CompilerVersion")]
    pub compiler_version: String,
    #[serde(rename = "EVMVersion", default)]
    pub evm_version: String,
    #[serde(rename = "ConstructorArguments", default)]
    pub constructor_arguments: String,
    #[serde(rename = "SwarmSource", default)]
    pub swarm_source: String,
    #[serde(rename = "OptimizationUsed")]
    pub optimization_used: String,
    #[serde(rename = "Runs")]
    pub runs: String,
    #[serde(rename = "LicenseType")]
    pub license_type: String,
    #[serde(rename = "Proxy")]
    pub proxy: String,
    #[serde(
        rename = "Implementation",
        deserialize_with = "empty_string_as_none",
        default
    )]
    pub implementation: Option<String>,
}

impl ContractSource {
    /// An upgradeable proxy: its effective implementation can change, so its
    /// `getsourcecode` response (which carries the implementation pointer) must
    /// not be cached permanently.
    pub fn is_proxy(&self) -> bool {
        self.proxy == "1"
    }

    /// Whether Etherscan returned verified source for this address. An
    /// unverified contract comes back with empty source and name — and may
    /// become verified later, so it shouldn't be cached permanently either.
    pub fn is_verified(&self) -> bool {
        !self.source_code.is_empty() || !self.contract_name.is_empty()
    }
}

/// Cache policy for a `getsourcecode` response (the first/only record).
///
/// Verified, non-proxy source is immutable → cache it forever. Proxies are
/// upgradeable and unverified contracts can still get verified → always
/// re-fetch so callers see the latest.
pub fn source_cache_policy(first: Option<&ContractSource>) -> CachePolicy {
    match first {
        Some(s) if s.is_proxy() => CachePolicy::Skip,
        Some(s) if s.is_verified() => CachePolicy::Permanent,
        _ => CachePolicy::Skip,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn verified_non_proxy() -> ContractSource {
        ContractSource {
            contract_name: "MyToken".into(),
            source_code: "contract MyToken {}".into(),
            proxy: "0".into(),
            ..Default::default()
        }
    }

    #[test]
    fn verified_non_proxy_is_cached_permanently() {
        let s = verified_non_proxy();
        assert!(s.is_verified() && !s.is_proxy());
        assert_eq!(source_cache_policy(Some(&s)), CachePolicy::Permanent);
    }

    #[test]
    fn proxy_is_never_cached_even_when_verified() {
        let mut s = verified_non_proxy();
        s.proxy = "1".into();
        assert!(s.is_proxy());
        // Upgradeable → always re-fetch so the implementation pointer is current.
        assert_eq!(source_cache_policy(Some(&s)), CachePolicy::Skip);
    }

    #[test]
    fn unverified_is_not_cached_permanently() {
        // Empty source + name = not (yet) verified; may become verified later.
        let s = ContractSource::default();
        assert!(!s.is_verified());
        assert_eq!(source_cache_policy(Some(&s)), CachePolicy::Skip);
    }

    #[test]
    fn missing_record_is_not_cached() {
        assert_eq!(source_cache_policy(None), CachePolicy::Skip);
    }
}
