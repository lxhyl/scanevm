use serde::de::DeserializeOwned;
use serde::Deserialize;
use crate::error::{AppError, Result};

const BASE_URL: &str = "https://api.etherscan.io/v2/api";

pub struct EtherscanClient {
    http: reqwest::Client,
    api_key: String,
    chain_id: u64,
}

#[derive(Deserialize)]
struct ApiEnvelope<T> {
    status: String,
    message: String,
    result: serde_json::Value,
    #[serde(skip)]
    _phantom: std::marker::PhantomData<T>,
}

#[derive(Deserialize)]
struct ProxyEnvelope {
    result: Option<serde_json::Value>,
    error: Option<ProxyError>,
}

#[derive(Deserialize)]
struct ProxyError {
    message: String,
}

impl EtherscanClient {
    pub fn new(api_key: String, chain_id: u64) -> Self {
        Self {
            http: reqwest::Client::new(),
            api_key,
            chain_id,
        }
    }

    /// REST API call — returns deserialized `result` field
    pub async fn call<T: DeserializeOwned>(
        &self,
        params: &[(&str, &str)],
    ) -> Result<T> {
        let chain_id = self.chain_id.to_string();
        let mut query: Vec<(&str, &str)> = vec![
            ("chainid", &chain_id),
            ("apikey", &self.api_key),
        ];
        query.extend_from_slice(params);

        let resp = self
            .http
            .get(BASE_URL)
            .query(&query)
            .send()
            .await?
            .error_for_status()?;

        let envelope: ApiEnvelope<T> = resp.json().await?;

        if envelope.status == "0" {
            let msg = envelope.message.clone();
            // Treat "no records" as empty — caller gets empty Vec/None
            if msg.contains("No transactions found")
                || msg.contains("No records found")
                || msg.contains("No token transfers found")
            {
                // Return empty JSON array / null so callers can deserialize as empty
                return Ok(serde_json::from_value(serde_json::json!([]))?);
            }
            let detail = envelope.result.as_str().unwrap_or(&msg).to_string();
            return Err(AppError::ApiError(detail));
        }

        Ok(serde_json::from_value(envelope.result)?)
    }

    /// JSON-RPC proxy call — returns deserialized `result` field
    pub async fn proxy<T: DeserializeOwned>(
        &self,
        method: &str,
        json_params: &str,
    ) -> Result<T> {
        let chain_id = self.chain_id.to_string();
        let query = [
            ("chainid", chain_id.as_str()),
            ("apikey", self.api_key.as_str()),
            ("module", "proxy"),
            ("action", method),
        ];

        // Build body: action doubles as the RPC method for Etherscan's proxy
        // Etherscan proxy uses query params, not JSON body
        let mut all_query: Vec<(&str, String)> = query
            .iter()
            .map(|(k, v)| (*k, v.to_string()))
            .collect();

        // Parse json_params as key=value pairs if non-empty
        if !json_params.is_empty() {
            // json_params is a JSON object like {"tag":"latest","boolean":true}
            if let Ok(map) = serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(json_params) {
                for (k, v) in &map {
                    let val = match v {
                        serde_json::Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    all_query.push((Box::leak(k.clone().into_boxed_str()), val));
                }
            }
        }

        let resp = self
            .http
            .get(BASE_URL)
            .query(&all_query)
            .send()
            .await?
            .error_for_status()?;

        let envelope: ProxyEnvelope = resp.json().await?;

        if let Some(err) = envelope.error {
            return Err(AppError::ApiError(err.message));
        }

        let result = envelope.result.unwrap_or(serde_json::Value::Null);
        Ok(serde_json::from_value(result)?)
    }
}
