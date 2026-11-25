//! Fetch a contract's verified source: the local cache, then the chain's
//! providers in order (port of the scan core's fetch).
//!
//! The runtime code is read first, so an EOA is reported as "no code" rather
//! than "not verified", and the codehash keys the cache. Providers are asked
//! in the chain's order until no later one could beat what was found (see
//! [`crate::sources::rank`]); one failing (rate limit, network) is recorded
//! and the next is asked. With no source anywhere, the report is "not
//! verified" if every provider answered, else the worst error.
//!
//! `--json` prints [`Report`] in the scan core's report format.

use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::cache::Cache;
use crate::chains::{ChainInfo, Provider};
use crate::client::EtherscanClient;
use crate::error::{Result, ScanevmError};
use crate::rpc::Reader;
use crate::sources::{
    best_possible_rank, blockscout, etherscan, sourcify, Compiler, Deployment, Fetched, Http,
    Outcome, ProxyHint, Quality, Target,
};
use crate::util::{assign_disk_paths, checksum, keccak_hex, now_rfc3339};

/// One provider asked, as reported in `attempts` / `providers_tried`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attempt {
    pub provider: Provider,
    /// found | not_verified | unsupported | skipped | error
    pub outcome: String,
    #[serde(rename = "match", default, skip_serializing_if = "Option::is_none")]
    pub quality: Option<Quality>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub ms: u64,
}

/// The runtime code at the address.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeRecord {
    pub chain_id: u64,
    pub address: String,
    /// keccak-256 of the runtime code; null when there is none.
    pub codehash: Option<String>,
    pub size: usize,
    /// The EIP-7702 delegation target when the code is a `0xef0100…` designator.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delegation: Option<String>,
    pub checked_at: String,
}

/// A source set as kept in the local cache.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Stored {
    pub fetched: Fetched,
    pub codehash: Option<String>,
    pub fetched_at: String,
    pub providers_tried: Vec<Attempt>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Cached,
    Fetched,
    NotVerified,
    NoCode,
    Error,
}

#[derive(Debug, Clone, Serialize)]
pub struct ErrorOut {
    pub kind: String,
    pub message: String,
}

pub struct Report {
    pub chain: ChainInfo,
    /// Lowercase `0x` address.
    pub address: String,
    pub status: Status,
    pub code: Option<CodeRecord>,
    pub source: Option<Stored>,
    pub attempts: Vec<Attempt>,
    pub error: Option<ErrorOut>,
}

impl Report {
    pub fn checksum(&self) -> String {
        checksum(&self.address)
    }

    /// The source, or the error that explains why there is none: not verified
    /// (exit 8), no code (7), or the worst provider trouble.
    pub fn into_source(self) -> Result<Stored> {
        let addr = self.checksum();
        match (self.status, self.source, self.error) {
            (_, Some(s), _) => Ok(s),
            (Status::NotVerified, None, _) => Err(ScanevmError::NotVerified(addr)),
            (Status::NoCode, None, Some(e)) => Err(ScanevmError::NotFound(e.message)),
            (_, None, Some(e)) => Err(match e.kind.as_str() {
                "rate_limited" => ScanevmError::RateLimited { retry_after: None },
                "network" => ScanevmError::Network(e.message),
                "invalid_key" => ScanevmError::InvalidApiKey,
                _ => ScanevmError::ApiError(e.message),
            }),
            (_, None, None) => Err(ScanevmError::NotVerified(addr)),
        }
    }
}

pub struct Fetcher<'a> {
    chain: ChainInfo,
    reader: &'a Reader<'a>,
    etherscan: Option<&'a EtherscanClient>,
    http: Http,
    cache: Cache,
}

impl<'a> Fetcher<'a> {
    pub fn new(
        chain: ChainInfo,
        reader: &'a Reader<'a>,
        etherscan: Option<&'a EtherscanClient>,
        no_cache: bool,
    ) -> Self {
        Self {
            chain,
            reader,
            etherscan,
            http: Http::new(),
            cache: Cache::new(no_cache),
        }
    }

    pub async fn fetch(&self, address: &str) -> Report {
        let address = address.trim().to_ascii_lowercase();
        let mut report = Report {
            chain: self.chain,
            address: address.clone(),
            status: Status::Error,
            code: None,
            source: None,
            attempts: Vec::new(),
            error: None,
        };

        report.code = self.code_record(&address).await;
        if let Some(code) = &report.code {
            if code.size == 0 || code.delegation.is_some() {
                report.status = Status::NoCode;
                let message = match &code.delegation {
                    Some(d) => format!(
                        "{} is an EOA delegating via EIP-7702 to {d}; fetch that address instead",
                        report.checksum()
                    ),
                    None => format!(
                        "no contract code at {} on {}",
                        report.checksum(),
                        self.chain.label()
                    ),
                };
                report.error = Some(ErrorOut {
                    kind: "not_found".into(),
                    message,
                });
                return report;
            }
        }
        let codehash = report.code.as_ref().and_then(|c| c.codehash.clone());
        let key = cache_key(self.chain.chain_id, &address, codehash.as_deref());
        if let Some(stored) = self
            .cache
            .get(&key)
            .and_then(|raw| serde_json::from_str::<Stored>(&raw).ok())
        {
            report.status = Status::Cached;
            report.source = Some(stored);
            return report;
        }

        let order = self.chain.sources;
        let mut best: Option<Fetched> = None;
        for (i, &p) in order.iter().enumerate() {
            if let Some(b) = &best {
                let ceiling = order[i..]
                    .iter()
                    .map(|&q| best_possible_rank(q))
                    .max()
                    .unwrap_or(0);
                if b.rank() >= ceiling {
                    break;
                }
            }
            let t0 = Instant::now();
            let res = self.ask(p, &address).await;
            let ms = t0.elapsed().as_millis() as u64;
            let mut a = Attempt {
                provider: p,
                outcome: String::new(),
                quality: None,
                error_kind: None,
                message: None,
                ms,
            };
            match res {
                Ok(Outcome::Found(f)) => {
                    a.outcome = "found".into();
                    a.quality = Some(f.quality);
                    a.message = Some(format!("{} ({} files)", f.name, f.sources.len()));
                    if best.as_ref().is_none_or(|b| f.rank() > b.rank()) {
                        best = Some(*f);
                    }
                }
                Ok(Outcome::NotVerified) => a.outcome = "not_verified".into(),
                Ok(Outcome::Unsupported(m)) => {
                    a.outcome = "unsupported".into();
                    a.message = Some(m);
                }
                Ok(Outcome::Skipped(m)) => {
                    a.outcome = "skipped".into();
                    a.message = Some(m);
                }
                Err(e) => {
                    a.outcome = "error".into();
                    a.error_kind = Some(error_kind(&e).into());
                    a.message = Some(e.to_string());
                }
            }
            report.attempts.push(a);
        }

        if let Some(fetched) = best {
            let stored = Stored {
                fetched,
                codehash,
                fetched_at: now_rfc3339(),
                providers_tried: report.attempts.clone(),
            };
            // A proxy's source is immutable, but the provider's proxy hint (its
            // implementation) is not: keep those fresh, as before.
            let is_proxy = stored
                .fetched
                .proxy_hint
                .as_ref()
                .is_some_and(|h| h.is_proxy);
            if !is_proxy {
                if let Ok(raw) = serde_json::to_string(&stored) {
                    self.cache.put(&key, &raw, None);
                }
            }
            report.status = Status::Fetched;
            report.source = Some(stored);
            return report;
        }

        let worst = report
            .attempts
            .iter()
            .filter_map(|a| a.error_kind.as_deref())
            .max_by_key(|k| severity(k));
        match worst {
            None => {
                report.status = Status::NotVerified;
                report.error = Some(ErrorOut {
                    kind: "not_verified".into(),
                    message: format!(
                        "{} is not verified on any provider{}",
                        report.checksum(),
                        report
                            .code
                            .as_ref()
                            .map(|c| format!(", {} bytes of code", c.size))
                            .unwrap_or_default()
                    ),
                });
            }
            Some(kind) => {
                let msgs: Vec<String> = report
                    .attempts
                    .iter()
                    .filter(|a| a.outcome == "error")
                    .map(|a| a.message.clone().unwrap_or_default())
                    .collect();
                report.error = Some(ErrorOut {
                    kind: kind.to_string(),
                    message: format!("no source found; {}", msgs.join("; ")),
                });
            }
        }
        report
    }

    async fn ask(&self, p: Provider, address: &str) -> Result<Outcome> {
        match p {
            Provider::Sourcify => sourcify::fetch(&self.http, &self.chain, address).await,
            Provider::Etherscan => {
                etherscan::fetch_etherscan(self.etherscan, &self.chain, address).await
            }
            Provider::Blockscout => blockscout::fetch(&self.http, &self.chain, address).await,
            Provider::Routescan => {
                etherscan::fetch_routescan(&self.http, &self.chain, address).await
            }
        }
    }

    /// Read the code (cached by the reader). `None` if it can't be read: the
    /// fetch then goes on without a codehash.
    async fn code_record(&self, address: &str) -> Option<CodeRecord> {
        let code = self.reader.code(address).await.ok()?;
        let delegation = (code.len() == 23 && code.starts_with(&[0xef, 0x01, 0x00]))
            .then(|| checksum(&format!("0x{}", crate::util::to_hex(&code[3..]))));
        Some(CodeRecord {
            chain_id: self.chain.chain_id,
            address: checksum(address),
            codehash: (!code.is_empty()).then(|| keccak_hex(&code)),
            size: code.len(),
            delegation,
            checked_at: now_rfc3339(),
        })
    }
}

fn cache_key(chain_id: u64, address: &str, codehash: Option<&str>) -> String {
    let ch = codehash
        .map(|h| {
            h.trim_start_matches("0x")
                .chars()
                .take(12)
                .collect::<String>()
        })
        .unwrap_or_else(|| "unknown".into());
    format!("src|{chain_id}|{address}|{ch}")
}

/// The error kinds reports use.
pub fn error_kind(e: &ScanevmError) -> &'static str {
    match e {
        ScanevmError::RateLimited { .. } => "rate_limited",
        ScanevmError::Network(_) => "network",
        ScanevmError::InvalidApiKey => "invalid_key",
        ScanevmError::NotFound(_) => "not_found",
        _ => "provider",
    }
}

fn severity(kind: &str) -> u8 {
    match kind {
        "rate_limited" => 5,
        "network" => 4,
        "invalid_key" => 3,
        "provider" => 2,
        _ => 1,
    }
}

// ---- the report as `--json` prints it (field order as the scan core's) ----

#[derive(Serialize)]
struct FileJson<'a> {
    path: &'a str,
    /// Where `--save` writes the file, relative to the directory.
    file: String,
    bytes: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    main: Option<bool>,
}

#[derive(Serialize)]
struct MetaJson<'a> {
    schema: u32,
    chain_id: u64,
    address: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    codehash: Option<&'a str>,
    name: &'a str,
    language: &'a str,
    compiler: &'a Compiler,
    #[serde(skip_serializing_if = "Option::is_none")]
    compilation_target: Option<&'a Target>,
    provider: Provider,
    #[serde(rename = "match")]
    quality: Quality,
    #[serde(skip_serializing_if = "Option::is_none")]
    similar_to: Option<&'a str>,
    source_url: &'a str,
    fetched_at: &'a str,
    providers_tried: &'a [Attempt],
    #[serde(skip_serializing_if = "Option::is_none")]
    constructor_args: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    proxy_hint: Option<&'a ProxyHint>,
    #[serde(skip_serializing_if = "Option::is_none")]
    deployment: Option<&'a Deployment>,
    #[serde(skip_serializing_if = "Option::is_none")]
    license: Option<&'a str>,
    files: Vec<FileJson<'a>>,
}

#[derive(Serialize)]
struct SourceJson<'a> {
    meta: MetaJson<'a>,
}

#[derive(Serialize)]
pub struct ReportJson<'a> {
    target: String,
    chain_id: u64,
    address: String,
    status: Status,
    #[serde(skip_serializing_if = "Option::is_none")]
    code: Option<&'a CodeRecord>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<SourceJson<'a>>,
    #[serde(skip_serializing_if = "<[Attempt]>::is_empty")]
    attempts: &'a [Attempt],
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<&'a ErrorOut>,
}

/// On-disk paths for a source set's files (what `--save` writes, and the
/// `file` field in `--json`).
pub fn disk_paths(f: &Fetched) -> Vec<String> {
    let originals: Vec<&str> = f.sources.iter().map(|(p, _)| p.as_str()).collect();
    assign_disk_paths(&originals)
}

pub fn report_json(r: &Report) -> ReportJson<'_> {
    let checksum = r.checksum();
    let source = r.source.as_ref().map(|s| {
        let f = &s.fetched;
        let main = f.target.as_ref().map(|t| t.path.as_str());
        let files = f
            .sources
            .iter()
            .zip(disk_paths(f))
            .map(|((path, content), file)| FileJson {
                path,
                file,
                bytes: content.len(),
                main: (Some(path.as_str()) == main).then_some(true),
            })
            .collect();
        SourceJson {
            meta: MetaJson {
                schema: 1,
                chain_id: r.chain.chain_id,
                address: checksum.clone(),
                codehash: s.codehash.as_deref(),
                name: &f.name,
                language: &f.language,
                compiler: &f.compiler,
                compilation_target: f.target.as_ref(),
                provider: f.provider,
                quality: f.quality,
                similar_to: f.similar_to.as_deref(),
                source_url: &f.url,
                fetched_at: &s.fetched_at,
                providers_tried: &s.providers_tried,
                constructor_args: f.constructor_args.as_deref(),
                proxy_hint: f.proxy_hint.as_ref(),
                deployment: f.deployment.as_ref(),
                license: f.license.as_deref(),
                files,
            },
        }
    });
    ReportJson {
        target: format!("{}:{checksum}", r.chain.name),
        chain_id: r.chain.chain_id,
        address: checksum,
        status: r.status,
        code: r.code.as_ref(),
        source,
        attempts: &r.attempts,
        error: r.error.as_ref(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chains::resolve_chain;

    fn fetched() -> Fetched {
        Fetched {
            provider: Provider::Sourcify,
            quality: Quality::Exact,
            similar_to: None,
            name: "Foo".into(),
            language: "solidity".into(),
            compiler: Compiler {
                version: "0.8.20".into(),
                ..Default::default()
            },
            target: Some(Target {
                path: "src/Foo.sol".into(),
                name: "Foo".into(),
            }),
            sources: vec![
                ("src/Foo.sol".into(), "contract Foo {}".into()),
                ("../evil.sol".into(), "x".into()),
            ],
            abi: None,
            constructor_args: None,
            proxy_hint: None,
            deployment: None,
            license: None,
            url: "https://sourcify.dev/server/v2/contract/1/0xab".into(),
        }
    }

    fn report(status: Status, source: Option<Stored>, error: Option<ErrorOut>) -> Report {
        Report {
            chain: resolve_chain("ethereum").unwrap(),
            address: "0x5aaeb6053f3e94c9b9a09f33669435e7ef1beaed".into(),
            status,
            code: None,
            source,
            attempts: vec![],
            error,
        }
    }

    #[test]
    fn json_report_has_the_core_shape() {
        let stored = Stored {
            fetched: fetched(),
            codehash: Some("0x12".into()),
            fetched_at: "2026-11-25T00:00:00Z".into(),
            providers_tried: vec![],
        };
        let r = report(Status::Fetched, Some(stored), None);
        let v = serde_json::to_value(report_json(&r)).unwrap();
        assert_eq!(
            v["target"],
            "ethereum:0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed"
        );
        assert_eq!(v["status"], "fetched");
        let meta = &v["source"]["meta"];
        assert_eq!(meta["match"], "exact");
        assert_eq!(meta["provider"], "sourcify");
        assert_eq!(meta["files"][0]["main"], true);
        assert_eq!(meta["files"][1]["file"], "__/evil.sol");
        assert!(v.get("attempts").is_none() && v.get("error").is_none());
    }

    #[test]
    fn reports_without_source_become_errors() {
        let nv = report(Status::NotVerified, None, None);
        assert_eq!(nv.into_source().unwrap_err().exit_code(), 8);
        let nc = report(
            Status::NoCode,
            None,
            Some(ErrorOut {
                kind: "not_found".into(),
                message: "no code".into(),
            }),
        );
        assert_eq!(nc.into_source().unwrap_err().exit_code(), 7);
        let rl = report(
            Status::Error,
            None,
            Some(ErrorOut {
                kind: "rate_limited".into(),
                message: "m".into(),
            }),
        );
        assert_eq!(rl.into_source().unwrap_err().exit_code(), 4);
    }

    #[test]
    fn cache_keys_use_the_codehash_prefix() {
        assert_eq!(
            cache_key(1, "0xab", Some("0x1234567890abcdef")),
            "src|1|0xab|1234567890ab"
        );
        assert_eq!(cache_key(1, "0xab", None), "src|1|0xab|unknown");
    }

    // Network test (keyless providers, public RPC): cargo test -- --ignored
    #[tokio::test]
    #[ignore]
    async fn fetches_multicall3_without_a_key() {
        let chain = resolve_chain("ethereum").unwrap();
        let reader = Reader::new(chain, None, true);
        let fetcher = Fetcher::new(chain, &reader, None, true);
        let r = fetcher
            .fetch("0xcA11bde05977b3631167028862bE2a173976CA11")
            .await;
        assert_eq!(r.status, Status::Fetched);
        assert_eq!(r.code.as_ref().unwrap().size, 3808);
        let s = r.into_source().unwrap();
        assert_eq!(
            (s.fetched.provider, s.fetched.quality),
            (Provider::Sourcify, Quality::Exact)
        );
        assert!(s.fetched.abi.is_some());
    }
}
