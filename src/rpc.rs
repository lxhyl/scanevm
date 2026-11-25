//! Reading chain state: code, storage slots, calls and event logs.
//!
//! Public JSON-RPC endpoints from the chain table come first: they need no key,
//! don't spend the Etherscan rate limit, and cover chains Etherscan no longer
//! serves. When every endpoint fails (or the chain has none listed) and an
//! Etherscan key is configured, the read goes through Etherscan's proxy module.
//!
//! Event logs are the other way round: Etherscan's logs API pages through the
//! whole history, while most public RPCs cap `eth_getLogs` ranges, so it goes
//! first when there is a key, and a single full-range `eth_getLogs` is the
//! fallback.

use std::time::Duration;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::cache::Cache;
use crate::chains::{ChainInfo, Provider};
use crate::client::EtherscanClient;
use crate::error::{Result, ScanevmError};

/// Storage slots can change (upgrades), so they're cached only briefly.
const SLOT_TTL: Duration = Duration::from_secs(60);
/// Etherscan `getLogs` returns at most this many records per page.
const GETLOGS_PAGE: usize = 1000;
/// Bound on Etherscan log pages, so a pathological contract can't loop forever.
const GETLOGS_MAX_PAGES: u32 = 20;

/// One event log (the same fields from Etherscan's logs API and `eth_getLogs`).
#[derive(Debug, Clone, Deserialize, Default)]
pub struct Log {
    #[serde(default)]
    pub data: String,
    #[serde(default)]
    pub topics: Vec<String>,
    #[serde(rename = "blockNumber", default)]
    pub block_number: String,
    #[serde(rename = "logIndex", default)]
    pub log_index: String,
}

/// Logs for one (address, topic0), and whether the list may be cut short.
pub struct Logs {
    pub logs: Vec<Log>,
    pub truncated: bool,
}

pub struct Reader<'a> {
    http: reqwest::Client,
    chain: ChainInfo,
    etherscan: Option<&'a EtherscanClient>,
    cache: Cache,
}

/// A JSON-RPC answer that is final (e.g. a reverted call), as opposed to an
/// endpoint that couldn't answer.
enum Answer {
    Result(Value),
    RpcError(String),
}

impl<'a> Reader<'a> {
    /// `etherscan` is the user's Etherscan client when a key is configured.
    pub fn new(chain: ChainInfo, etherscan: Option<&'a EtherscanClient>, no_cache: bool) -> Self {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(20))
            .user_agent(concat!("scanevm/", env!("CARGO_PKG_VERSION")))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            http,
            chain,
            etherscan: etherscan.filter(|_| etherscan_serves(&chain)),
            cache: Cache::new(no_cache),
        }
    }

    /// Runtime code at `address` (empty for an EOA). Code never changes once
    /// deployed, so non-empty code is cached for good.
    pub async fn code(&self, address: &str) -> Result<Vec<u8>> {
        let key = format!("rpc|{}|code|{address}", self.chain.chain_id);
        if let Some(hex) = self.cache.get(&key) {
            if let Some(b) = from_hex(&hex) {
                return Ok(b);
            }
        }
        let v = self
            .read(
                "eth_getCode",
                json!([address, "latest"]),
                &format!(r#"{{"address":"{address}","tag":"latest"}}"#),
            )
            .await?;
        let hex = v.as_str().unwrap_or("0x").to_string();
        let bytes = from_hex(&hex).ok_or_else(|| bad_answer("eth_getCode"))?;
        if !bytes.is_empty() {
            self.cache.put(&key, &hex, None);
        }
        Ok(bytes)
    }

    /// The 32-byte storage word at `slot`, as `0x`-hex.
    pub async fn storage(&self, address: &str, slot: &str) -> Result<String> {
        let key = format!("rpc|{}|slot|{address}|{slot}", self.chain.chain_id);
        if let Some(w) = self.cache.get(&key) {
            return Ok(w);
        }
        let v = self
            .read(
                "eth_getStorageAt",
                json!([address, slot, "latest"]),
                &format!(r#"{{"address":"{address}","position":"{slot}","tag":"latest"}}"#),
            )
            .await?;
        let w = v
            .as_str()
            .ok_or_else(|| bad_answer("eth_getStorageAt"))?
            .to_string();
        self.cache.put(&key, &w, Some(SLOT_TTL));
        Ok(w)
    }

    /// `eth_call` at the latest block; a revert is an error.
    pub async fn call(&self, to: &str, data: &str) -> Result<Vec<u8>> {
        let v = self
            .read(
                "eth_call",
                json!([{ "to": to, "data": data }, "latest"]),
                &format!(r#"{{"to":"{to}","data":"{data}","tag":"latest"}}"#),
            )
            .await?;
        v.as_str()
            .and_then(from_hex)
            .ok_or_else(|| bad_answer("eth_call"))
    }

    /// Every log `address` emitted with `topic0`, oldest first.
    pub async fn logs(&self, address: &str, topic0: &str) -> Result<Logs> {
        let mut errors = Vec::new();
        if let Some(es) = self.etherscan {
            match etherscan_logs(es, address, topic0).await {
                Ok(l) => return Ok(l),
                Err(e) => errors.push(format!("etherscan: {e}")),
            }
        }
        let filter = json!([{ "address": address, "topics": [topic0], "fromBlock": "0x0", "toBlock": "latest" }]);
        match self.rpc("eth_getLogs", filter).await {
            Ok(Answer::Result(v)) => {
                let mut logs: Vec<Log> = serde_json::from_value(v).unwrap_or_default();
                sort_logs(&mut logs);
                return Ok(Logs {
                    logs,
                    truncated: false,
                });
            }
            Ok(Answer::RpcError(m)) => errors.push(format!("rpc: {m}")),
            Err(e) => errors.push(format!("rpc: {e}")),
        }
        Err(ScanevmError::Network(format!(
            "couldn't read event logs ({})",
            errors.join("; ")
        )))
    }

    /// A state read: the chain's public RPCs, then Etherscan's proxy module.
    async fn read(&self, method: &str, params: Value, etherscan_params: &str) -> Result<Value> {
        let rpc_err = match self.rpc(method, params).await {
            Ok(Answer::Result(v)) => return Ok(v),
            // The node answered; another one would answer the same.
            Ok(Answer::RpcError(m)) => {
                return Err(ScanevmError::ApiError(format!("{method}: {m}")))
            }
            Err(e) => e,
        };
        match self.etherscan {
            Some(es) => es.proxy::<Value>(method, etherscan_params).await,
            None => Err(rpc_err),
        }
    }

    /// Ask each endpoint in turn until one answers. An `Err` means none could
    /// (or there are none).
    async fn rpc(&self, method: &str, params: Value) -> Result<Answer> {
        let body = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
        let mut last = format!("no public RPC listed for {}", self.chain.label());
        for url in self.chain.rpc {
            let host = host(url);
            let resp = match self.http.post(*url).json(&body).send().await {
                Ok(r) => r,
                Err(e) => {
                    last = format!("{host}: {}", ScanevmError::from_http(e));
                    continue;
                }
            };
            let status = resp.status();
            let text = match resp.text().await {
                Ok(t) => t,
                Err(e) => {
                    last = format!("{host}: {}", ScanevmError::from_http(e));
                    continue;
                }
            };
            let Ok(v) = serde_json::from_str::<Value>(&text) else {
                last = format!("{host}: HTTP {} (not JSON-RPC)", status.as_u16());
                continue;
            };
            if let Some(r) = v.get("result") {
                return Ok(Answer::Result(r.clone()));
            }
            let msg = v["error"]["message"]
                .as_str()
                .unwrap_or("unknown error")
                .to_string();
            if is_endpoint_trouble(status.as_u16(), &msg) {
                last = format!("{host}: {msg}");
                continue;
            }
            return Ok(Answer::RpcError(msg));
        }
        Err(ScanevmError::Network(last))
    }
}

/// Whether Etherscan's API covers this chain (listed with it, or not in the
/// table at all, where it may well).
fn etherscan_serves(chain: &ChainInfo) -> bool {
    !chain.known || chain.sources.contains(&Provider::Etherscan)
}

/// Rate limits, plan limits and server trouble say nothing about the answer:
/// the next endpoint may do better.
fn is_endpoint_trouble(status: u16, msg: &str) -> bool {
    let m = msg.to_lowercase();
    status == 429
        || status >= 500
        || m.contains("rate limit")
        || m.contains("too many")
        || m.contains("limit exceeded")
        || m.contains("paid plan")
        || m.contains("upgrade")
        || m.contains("unauthorized")
        || m.contains("api key")
        || m.contains("timeout")
        || m.contains("temporarily")
}

/// Page through Etherscan's logs API (1000 per page).
async fn etherscan_logs(es: &EtherscanClient, address: &str, topic0: &str) -> Result<Logs> {
    let mut logs: Vec<Log> = Vec::new();
    let mut truncated = false;
    let offset = GETLOGS_PAGE.to_string();
    for page in 1..=GETLOGS_MAX_PAGES {
        let page_str = page.to_string();
        let batch: Vec<Log> = es
            .call(&[
                ("module", "logs"),
                ("action", "getLogs"),
                ("address", address),
                ("fromBlock", "0"),
                ("toBlock", "latest"),
                ("topic0", topic0),
                ("page", &page_str),
                ("offset", &offset),
            ])
            .await?;
        let n = batch.len();
        logs.extend(batch);
        if n < GETLOGS_PAGE {
            break;
        }
        if page == GETLOGS_MAX_PAGES {
            truncated = true;
        }
    }
    sort_logs(&mut logs);
    Ok(Logs { logs, truncated })
}

fn sort_logs(logs: &mut [Log]) {
    logs.sort_by_key(|l| (hex_to_u64(&l.block_number), hex_to_u64(&l.log_index)));
}

fn bad_answer(method: &str) -> ScanevmError {
    ScanevmError::ApiError(format!("{method}: unexpected answer"))
}

/// The host of an RPC URL, for messages (URLs may carry keys in their path).
fn host(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    rest.split(['/', '?']).next().unwrap_or(rest)
}

/// Decode `0x`-hex (or bare hex) into bytes; `None` if malformed.
pub fn from_hex(s: &str) -> Option<Vec<u8>> {
    let h = s.trim().trim_start_matches("0x");
    if h.len() % 2 != 0 {
        return None;
    }
    (0..h.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(h.get(i..i + 2)?, 16).ok())
        .collect()
}

/// Parse a `0x`-hex (or decimal) quantity; 0 if malformed.
pub fn hex_to_u64(s: &str) -> u64 {
    let t = s.trim();
    match t.strip_prefix("0x") {
        Some(h) if h.is_empty() => 0,
        Some(h) => u64::from_str_radix(h, 16).unwrap_or(0),
        None => t.parse().unwrap_or(0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chains::resolve_chain;

    #[test]
    fn hex_helpers() {
        assert_eq!(from_hex("0x"), Some(vec![]));
        assert_eq!(from_hex("0x00ff"), Some(vec![0, 255]));
        assert_eq!(from_hex("0xabc"), None);
        assert_eq!(from_hex("0xzz"), None);
        assert_eq!(hex_to_u64("0x1a"), 26);
        assert_eq!(hex_to_u64("0x"), 0);
        assert_eq!(hex_to_u64("42"), 42);
    }

    #[test]
    fn host_hides_paths() {
        assert_eq!(host("https://rpc.example.org/v1/SECRET"), "rpc.example.org");
        assert_eq!(host("https://bsc.example:8545"), "bsc.example:8545");
    }

    #[test]
    fn endpoint_trouble_is_retried_elsewhere() {
        assert!(is_endpoint_trouble(429, "x"));
        assert!(is_endpoint_trouble(
            200,
            "chain is not available on free plan, please upgrade to paid plan"
        ));
        assert!(!is_endpoint_trouble(200, "execution reverted"));
    }

    #[test]
    fn etherscan_only_where_it_serves() {
        assert!(etherscan_serves(&resolve_chain("ethereum").unwrap()));
        assert!(!etherscan_serves(&resolve_chain("scroll").unwrap()));
        assert!(etherscan_serves(&resolve_chain("7777777").unwrap()));
    }

    #[test]
    fn logs_sort_by_block_then_index() {
        let mut v = vec![
            Log {
                block_number: "0x2".into(),
                log_index: "0x0".into(),
                ..Default::default()
            },
            Log {
                block_number: "0x1".into(),
                log_index: "0x5".into(),
                ..Default::default()
            },
            Log {
                block_number: "0x1".into(),
                log_index: "0x1".into(),
                ..Default::default()
            },
        ];
        sort_logs(&mut v);
        let order: Vec<_> = v
            .iter()
            .map(|l| (l.block_number.as_str(), l.log_index.as_str()))
            .collect();
        assert_eq!(order, [("0x1", "0x1"), ("0x1", "0x5"), ("0x2", "0x0")]);
    }

    // Network tests (public RPCs, no key): cargo test -- --ignored
    #[tokio::test]
    #[ignore]
    async fn reads_code_and_slots_over_public_rpc() {
        let reader = Reader::new(resolve_chain("ethereum").unwrap(), None, true);
        let usdc = "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48";
        assert!(!reader.code(usdc).await.unwrap().is_empty());
        // FiatTokenProxy keeps its implementation in the legacy zeppelinos slot.
        let slot = "0x7050c9e0f4ca769c69bd3a8ef740bc37934f8e2c036e5a723fd8ee048ed3f8c3";
        let w = reader.storage(usdc, slot).await.unwrap();
        assert!(
            w.to_lowercase()
                .ends_with("43506849d7c04f9138d1a2050bbf3a0c054402dd"),
            "{w}"
        );
    }

    #[tokio::test]
    #[ignore]
    async fn reads_diamond_facets_over_public_rpc() {
        let reader = Reader::new(resolve_chain("ethereum").unwrap(), None, true);
        // LI.FI diamond: DiamondLoupe facets().
        let lifi = "0x1231deb6f5749ef6ce6943a275a1d3e7486f4eae";
        let f = crate::diamond::resolve_facets(&reader, lifi)
            .await
            .unwrap()
            .unwrap();
        assert!(f.facets.len() > 10, "{} facets", f.facets.len());
    }
}
