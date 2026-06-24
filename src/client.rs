use std::sync::Mutex;
use std::time::{Duration, Instant};

use reqwest::StatusCode;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tokio::time::sleep;

use crate::cache::Cache;
use crate::error::{Result, ScanevmError};

const BASE_URL: &str = "https://api.etherscan.io/v2/api";
/// Retry transient failures (timeouts, 429, 5xx, status=0 rate-limit) this many times.
const MAX_RETRIES: u32 = 3;
/// Minimum spacing between requests *per key* — stays under Etherscan's free
/// 5 req/s ceiling. With N keys in the pool the aggregate ceiling is ~N× this.
const MIN_REQUEST_INTERVAL: Duration = Duration::from_millis(220);

/// How to cache a freshly-fetched value, decided *from the value itself* by
/// [`EtherscanClient::call_cached_with`]. Lets a response be cached permanently
/// only when it is known-immutable (e.g. a verified, non-proxy contract) while
/// upgradeable or not-yet-final responses are always re-fetched.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CachePolicy {
    /// Cache forever — the data can never change for this key.
    Permanent,
    /// Never cache — always hit the network (and ignore any stale entry).
    Skip,
}

/// Etherscan REST `message` values that mean "the query was fine, there is just
/// no data" — callers should get an empty collection rather than an error.
const NO_RECORDS: &[&str] = &[
    "No transactions found",
    "No records found",
    "No token transfers found",
];

pub struct EtherscanClient {
    http: reqwest::Client,
    /// The API key pool. Each request picks one key (see [`Self::throttle`]);
    /// `apikey` is appended to the query at send time, not baked in earlier.
    keys: Vec<String>,
    chain_id: u64,
    cache: Cache,
    /// Per-key reservation timeline: `schedule[i]` is the earliest instant
    /// `keys[i]` may next be used. Guards client-side throttling so each key
    /// independently stays under the rate limit, and lets a rate-limited key be
    /// cooled down (see [`Self::penalize`]) so retries rotate to a fresh one.
    schedule: Mutex<Vec<Instant>>,
}

#[derive(Deserialize)]
struct ApiEnvelope {
    status: String,
    message: String,
    result: serde_json::Value,
}

impl EtherscanClient {
    /// Build a client over a pool of one or more API keys. Callers obtain the
    /// pool from [`crate::config::Config::require_keys`], which guarantees it is
    /// non-empty; an empty pool would make every request fail. `no_cache`
    /// (the `--no-cache` flag) disables the local response cache for this run.
    pub fn new(keys: Vec<String>, chain_id: u64, no_cache: bool) -> Self {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .user_agent(concat!("scanevm/", env!("CARGO_PKG_VERSION")))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        // All keys start free now; ties resolve to the lowest index so a single
        // key behaves exactly as before.
        let schedule = vec![Instant::now(); keys.len()];
        Self {
            http,
            keys,
            chain_id,
            cache: Cache::new(no_cache),
            schedule: Mutex::new(schedule),
        }
    }

    /// REST API call — returns the deserialized `result` field.
    pub async fn call<T: DeserializeOwned>(&self, params: &[(&str, &str)]) -> Result<T> {
        let body = self.send(&self.base_query(params)).await?;
        parse_envelope(&body)
    }

    /// Like [`call`](Self::call), but reads/writes a local cache with the given
    /// TTL (`None` = cache permanently). The key is derived from the chain id
    /// and request params, so different addresses/actions never collide.
    pub async fn call_cached<T: DeserializeOwned + Serialize>(
        &self,
        ttl: Option<Duration>,
        params: &[(&str, &str)],
    ) -> Result<T> {
        let key = call_key(self.chain_id, params);
        if let Some(raw) = self.cache.get(&key) {
            if let Ok(value) = serde_json::from_str::<T>(&raw) {
                return Ok(value);
            }
        }
        let value: T = self.call(params).await?;
        if let Ok(serialized) = serde_json::to_string(&value) {
            self.cache.put(&key, &serialized, ttl);
        }
        Ok(value)
    }

    /// Like [`call_cached`](Self::call_cached), but the cache lifetime is chosen
    /// from the fetched value via `policy`, so a single endpoint can be cached
    /// permanently for immutable results yet re-fetched for upgradeable ones.
    ///
    /// `policy` is consulted on the cached value too: a cache entry whose value
    /// now resolves to [`CachePolicy::Skip`] is ignored and re-fetched, so even
    /// entries written by an older build (before the policy existed) can't pin
    /// stale data for an upgradeable contract.
    pub async fn call_cached_with<T, F>(&self, params: &[(&str, &str)], policy: F) -> Result<T>
    where
        T: DeserializeOwned + Serialize,
        F: Fn(&T) -> CachePolicy,
    {
        let key = call_key(self.chain_id, params);
        if let Some(raw) = self.cache.get(&key) {
            if let Ok(value) = serde_json::from_str::<T>(&raw) {
                if policy(&value) == CachePolicy::Permanent {
                    return Ok(value);
                }
            }
        }
        let value: T = self.call(params).await?;
        if policy(&value) == CachePolicy::Permanent {
            if let Ok(serialized) = serde_json::to_string(&value) {
                self.cache.put(&key, &serialized, None);
            }
        }
        Ok(value)
    }

    /// JSON-RPC proxy call — returns the deserialized `result` field.
    pub async fn proxy<T: DeserializeOwned>(&self, method: &str, json_params: &str) -> Result<T> {
        let value = self.proxy_value(method, json_params).await?;
        Ok(serde_json::from_value(value)?)
    }

    /// Cached variant of [`proxy`](Self::proxy).
    pub async fn proxy_cached<T: DeserializeOwned + Serialize>(
        &self,
        ttl: Option<Duration>,
        method: &str,
        json_params: &str,
    ) -> Result<T> {
        let key = proxy_key(self.chain_id, method, json_params);
        if let Some(raw) = self.cache.get(&key) {
            if let Ok(value) = serde_json::from_str::<T>(&raw) {
                return Ok(value);
            }
        }
        let value: T = self.proxy(method, json_params).await?;
        if let Ok(serialized) = serde_json::to_string(&value) {
            self.cache.put(&key, &serialized, ttl);
        }
        Ok(value)
    }

    /// Read a manually-keyed cache entry (for callers that compose several
    /// requests into one cached payload, e.g. `tx`).
    pub fn cache_get(&self, parts: &[&str]) -> Option<String> {
        self.cache.get(&manual_key(self.chain_id, parts))
    }

    /// Write a manually-keyed cache entry. `None` TTL caches permanently.
    pub fn cache_put(&self, parts: &[&str], data: &str, ttl: Option<Duration>) {
        self.cache.put(&manual_key(self.chain_id, parts), data, ttl);
    }

    async fn proxy_value(&self, method: &str, json_params: &str) -> Result<serde_json::Value> {
        let body = self.send(&self.proxy_query(method, json_params)).await?;
        parse_proxy(&body)
    }

    fn base_query(&self, params: &[(&str, &str)]) -> Vec<(String, String)> {
        // `apikey` is appended later, in `send`, once a pool key is chosen.
        let mut query = vec![("chainid".to_string(), self.chain_id.to_string())];
        query.extend(params.iter().map(|(k, v)| (k.to_string(), v.to_string())));
        query
    }

    fn proxy_query(&self, method: &str, json_params: &str) -> Vec<(String, String)> {
        // Etherscan's proxy module uses query params, not a JSON body;
        // `action` doubles as the JSON-RPC method name. `apikey` is appended
        // later, in `send`, once a pool key is chosen.
        let mut query = vec![
            ("chainid".to_string(), self.chain_id.to_string()),
            ("module".to_string(), "proxy".to_string()),
            ("action".to_string(), method.to_string()),
        ];
        // json_params is a JSON object like {"tag":"latest","boolean":true};
        // flatten it into additional query params.
        if !json_params.is_empty() {
            if let Ok(map) =
                serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(json_params)
            {
                for (k, v) in &map {
                    let val = match v {
                        serde_json::Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    query.push((k.clone(), val));
                }
            }
        }
        query
    }

    /// Send a request, retrying transient failures with backoff, and return the
    /// raw response body. The URL (which carries the API key) is never surfaced.
    ///
    /// Each attempt picks a pool key via [`Self::throttle`] and appends it as
    /// `apikey`. On a rate-limit response the offending key is cooled down (see
    /// [`Self::penalize`]) before retrying, so with more than one key the retry
    /// rotates to a fresh key immediately; with a single key the cooldown simply
    /// becomes the backoff wait, preserving the original behavior.
    async fn send(&self, query: &[(String, String)]) -> Result<String> {
        let mut attempt: u32 = 0;
        loop {
            let idx = self.throttle().await;
            let mut q = Vec::with_capacity(query.len() + 1);
            q.extend_from_slice(query);
            q.push(("apikey".to_string(), self.keys[idx].clone()));
            match self.http.get(BASE_URL).query(&q).send().await {
                Ok(resp) => {
                    let status = resp.status();
                    if status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
                        let retry_after = retry_after_secs(&resp);
                        if attempt < MAX_RETRIES {
                            attempt += 1;
                            self.penalize(idx, backoff_delay(attempt, retry_after));
                            continue;
                        }
                        return Err(if status == StatusCode::TOO_MANY_REQUESTS {
                            ScanevmError::RateLimited { retry_after }
                        } else {
                            ScanevmError::Network(format!("HTTP {}", status.as_u16()))
                        });
                    }
                    let body = resp.text().await.map_err(ScanevmError::from_http)?;
                    // Etherscan also signals rate limiting via a 200 + status=0 body.
                    if is_rate_limited_body(&body) && attempt < MAX_RETRIES {
                        attempt += 1;
                        self.penalize(idx, backoff_delay(attempt, None));
                        continue;
                    }
                    return Ok(body);
                }
                Err(e) => {
                    if (e.is_timeout() || e.is_connect() || e.is_request()) && attempt < MAX_RETRIES
                    {
                        attempt += 1;
                        sleep(backoff_delay(attempt, None)).await;
                        continue;
                    }
                    return Err(ScanevmError::from_http(e));
                }
            }
        }
    }

    /// Reserve the next free key in the pool, sleeping until its slot opens, and
    /// return its index. Picking the soonest-free key under the lock keeps
    /// concurrent calls (e.g. `tokio::join!`) spread across keys and each key
    /// individually below the rate limit.
    async fn throttle(&self) -> usize {
        let (idx, wait) = {
            let mut sched = self.schedule.lock().unwrap();
            reserve_slot(&mut sched, Instant::now())
        };
        if !wait.is_zero() {
            sleep(wait).await;
        }
        idx
    }

    /// Push `keys[idx]`'s next-free instant out by `delay` (a no-op if it is
    /// already further out). After a rate-limit response this steers the next
    /// [`Self::throttle`] toward a different, cooler key.
    fn penalize(&self, idx: usize, delay: Duration) {
        let mut sched = self.schedule.lock().unwrap();
        let until = Instant::now() + delay;
        if sched[idx] < until {
            sched[idx] = until;
        }
    }
}

/// Pick the key whose slot frees up soonest, reserve it (advancing its slot by
/// `MIN_REQUEST_INTERVAL` from whichever is later, the slot or `now`), and
/// return its index plus how long the caller must wait before using it.
///
/// Pulled out as a pure function so the scheduling can be unit-tested without
/// real time. `sched` must be non-empty.
fn reserve_slot(sched: &mut [Instant], now: Instant) -> (usize, Duration) {
    let idx = (0..sched.len()).min_by_key(|&i| sched[i]).unwrap_or(0);
    let earliest = sched[idx].max(now);
    sched[idx] = earliest + MIN_REQUEST_INTERVAL;
    (idx, earliest.saturating_duration_since(now))
}

fn parse_envelope<T: DeserializeOwned>(body: &str) -> Result<T> {
    let env: ApiEnvelope = serde_json::from_str(body)?;
    if env.status == "0" {
        let msg = env.message.trim();
        if is_no_records(msg) {
            return empty_value::<T>();
        }
        let detail = env
            .result
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| msg.to_string());
        return Err(classify_status0(&detail));
    }
    Ok(serde_json::from_value(env.result)?)
}

fn parse_proxy(body: &str) -> Result<serde_json::Value> {
    use serde_json::Value;
    let v: Value = serde_json::from_str(body)?;
    // JSON-RPC error object.
    if let Some(err) = v.get("error") {
        let msg = err
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("proxy error")
            .to_string();
        return Err(ScanevmError::ApiError(msg));
    }
    // Etherscan top-level error envelope (e.g. rate limit / bad key) on the proxy path.
    if v.get("status").and_then(|s| s.as_str()) == Some("0") {
        let detail = v
            .get("result")
            .and_then(|r| r.as_str())
            .or_else(|| v.get("message").and_then(|m| m.as_str()))
            .unwrap_or("API error")
            .to_string();
        return Err(classify_status0(&detail));
    }
    Ok(v.get("result").cloned().unwrap_or(Value::Null))
}

fn is_no_records(msg: &str) -> bool {
    NO_RECORDS.iter().any(|m| msg.contains(m))
}

/// Turn an empty result into the natural empty value for `T`: `[]` for
/// collections, `""` for strings, `null` for options — fixing the bug where a
/// blanket `[]` failed to deserialize into a `String` result (balance, abi).
fn empty_value<T: DeserializeOwned>() -> Result<T> {
    use serde_json::Value;
    for candidate in [
        Value::Array(vec![]),
        Value::String(String::new()),
        Value::Null,
    ] {
        if let Ok(value) = serde_json::from_value::<T>(candidate) {
            return Ok(value);
        }
    }
    Err(ScanevmError::ApiError("no records found".to_string()))
}

fn classify_status0(detail: &str) -> ScanevmError {
    let lower = detail.to_lowercase();
    if lower.contains("invalid api key") {
        ScanevmError::InvalidApiKey
    } else if lower.contains("rate limit")
        || lower.contains("max calls")
        || lower.contains("too many")
    {
        ScanevmError::RateLimited { retry_after: None }
    } else {
        ScanevmError::ApiError(detail.to_string())
    }
}

fn is_rate_limited_body(body: &str) -> bool {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(body) else {
        return false;
    };
    if v.get("status").and_then(|s| s.as_str()) != Some("0") {
        return false;
    }
    let msg = v.get("message").and_then(|m| m.as_str()).unwrap_or("");
    let res = v.get("result").and_then(|r| r.as_str()).unwrap_or("");
    let hay = format!("{msg} {res}").to_lowercase();
    hay.contains("rate limit") || hay.contains("max calls") || hay.contains("too many")
}

fn retry_after_secs(resp: &reqwest::Response) -> Option<u64> {
    resp.headers()
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
}

fn backoff_delay(attempt: u32, retry_after: Option<u64>) -> Duration {
    if let Some(secs) = retry_after {
        return Duration::from_secs(secs.min(30));
    }
    // 250ms, 500ms, 1s, ... capped at 4s.
    let shift = attempt.clamp(1, 5) - 1;
    let millis = 250u64.saturating_mul(1u64 << shift);
    Duration::from_millis(millis.min(4000))
}

fn call_key(chain_id: u64, params: &[(&str, &str)]) -> String {
    let mut parts: Vec<String> = params
        .iter()
        .filter(|(k, _)| *k != "apikey")
        .map(|(k, v)| format!("{k}={v}"))
        .collect();
    parts.sort();
    format!("call|{chain_id}|{}", parts.join("&"))
}

fn proxy_key(chain_id: u64, method: &str, json_params: &str) -> String {
    format!("proxy|{chain_id}|{method}|{json_params}")
}

fn manual_key(chain_id: u64, parts: &[&str]) -> String {
    format!("manual|{chain_id}|{}", parts.join("|"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn success_envelope_deserializes_result() {
        let body = r#"{"status":"1","message":"OK","result":"12345"}"#;
        let v: String = parse_envelope(body).unwrap();
        assert_eq!(v, "12345");
    }

    #[test]
    fn no_records_yields_empty_vec() {
        let body = r#"{"status":"0","message":"No transactions found","result":[]}"#;
        let v: Vec<String> = parse_envelope(body).unwrap();
        assert!(v.is_empty());
    }

    #[test]
    fn no_records_yields_empty_string_for_scalar_result() {
        // Regression: a blanket `[]` used to fail to deserialize into String.
        let body = r#"{"status":"0","message":"No records found","result":"0"}"#;
        let v: String = parse_envelope(body).unwrap();
        assert_eq!(v, "");
    }

    #[test]
    fn invalid_api_key_is_classified() {
        let body = r#"{"status":"0","message":"NOTOK","result":"Invalid API Key"}"#;
        let err = parse_envelope::<String>(body).unwrap_err();
        assert!(matches!(err, ScanevmError::InvalidApiKey));
    }

    #[test]
    fn rate_limit_is_classified() {
        let body = r#"{"status":"0","message":"NOTOK","result":"Max rate limit reached"}"#;
        let err = parse_envelope::<String>(body).unwrap_err();
        assert!(matches!(err, ScanevmError::RateLimited { .. }));
    }

    #[test]
    fn generic_api_error_passes_through() {
        let body = r#"{"status":"0","message":"NOTOK","result":"Something specific went wrong"}"#;
        let err = parse_envelope::<String>(body).unwrap_err();
        match err {
            ScanevmError::ApiError(d) => assert!(d.contains("Something specific")),
            other => panic!("expected ApiError, got {other:?}"),
        }
    }

    #[test]
    fn rate_limited_body_detected() {
        assert!(is_rate_limited_body(
            r#"{"status":"0","message":"NOTOK","result":"Max calls per sec rate limit reached"}"#
        ));
        assert!(!is_rate_limited_body(
            r#"{"status":"1","message":"OK","result":"x"}"#
        ));
    }

    #[test]
    fn proxy_null_result_is_null() {
        let v = parse_proxy(r#"{"jsonrpc":"2.0","id":1,"result":null}"#).unwrap();
        assert!(v.is_null());
    }

    #[test]
    fn proxy_error_is_api_error() {
        let err = parse_proxy(r#"{"jsonrpc":"2.0","id":1,"error":{"message":"bad"}}"#).unwrap_err();
        assert!(matches!(err, ScanevmError::ApiError(_)));
    }

    #[test]
    fn backoff_honors_retry_after_and_grows() {
        assert_eq!(backoff_delay(1, Some(7)), Duration::from_secs(7));
        assert_eq!(backoff_delay(1, None), Duration::from_millis(250));
        assert_eq!(backoff_delay(2, None), Duration::from_millis(500));
        assert_eq!(backoff_delay(3, None), Duration::from_millis(1000));
        assert!(backoff_delay(10, None) <= Duration::from_millis(4000));
    }

    #[test]
    fn single_key_throttles_to_fixed_cadence() {
        // One key always reserves index 0 and spaces requests by the interval.
        let base = Instant::now();
        let mut sched = vec![base];
        assert_eq!(reserve_slot(&mut sched, base), (0, Duration::ZERO));
        assert_eq!(reserve_slot(&mut sched, base), (0, MIN_REQUEST_INTERVAL));
        assert_eq!(reserve_slot(&mut sched, base), (0, 2 * MIN_REQUEST_INTERVAL));
    }

    #[test]
    fn two_keys_burst_then_alternate() {
        // Two keys let the first two requests fire immediately (a 2× burst),
        // then alternate at the per-key interval — i.e. ~2× the throughput.
        let base = Instant::now();
        let mut sched = vec![base, base];
        assert_eq!(reserve_slot(&mut sched, base), (0, Duration::ZERO));
        assert_eq!(reserve_slot(&mut sched, base), (1, Duration::ZERO));
        assert_eq!(reserve_slot(&mut sched, base), (0, MIN_REQUEST_INTERVAL));
        assert_eq!(reserve_slot(&mut sched, base), (1, MIN_REQUEST_INTERVAL));
    }

    #[test]
    fn reserve_skips_a_penalized_key() {
        // A key cooled down (as `penalize` does) is passed over in favor of a
        // free one, with no wait — the rate-limit-rotation path.
        let base = Instant::now();
        let mut sched = vec![base + Duration::from_secs(10), base];
        assert_eq!(reserve_slot(&mut sched, base), (1, Duration::ZERO));
    }

    #[test]
    fn cache_keys_are_stable_and_distinct() {
        let a = call_key(
            1,
            &[
                ("module", "account"),
                ("action", "balance"),
                ("address", "0xA"),
            ],
        );
        let b = call_key(
            1,
            &[
                ("address", "0xA"),
                ("action", "balance"),
                ("module", "account"),
            ],
        );
        assert_eq!(a, b, "key must be order-independent");
        let c = call_key(
            1,
            &[
                ("module", "account"),
                ("action", "balance"),
                ("address", "0xB"),
            ],
        );
        assert_ne!(a, c, "different address must differ");
        let d = call_key(
            137,
            &[
                ("module", "account"),
                ("action", "balance"),
                ("address", "0xA"),
            ],
        );
        assert_ne!(a, d, "different chain must differ");
    }
}
