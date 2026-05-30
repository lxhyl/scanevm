use crate::chains::resolve_chain;
use crate::client::EtherscanClient;
use crate::config::{Config, OutputFormat};
use crate::error::{Result, ScanevmError};
use crate::output::{format_eth, format_gwei, format_timestamp, print_json, print_kv_table};
use clap::Args;
use serde_json::Value;

#[derive(Debug, Args)]
pub struct TxArgs {
    /// Transaction hash
    pub hash: String,

    /// Chain name or ID (e.g. ethereum, polygon, base, 137)
    #[arg(long, short = 'c')]
    pub chain: String,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

pub async fn run(args: &TxArgs, cfg: &Config) -> Result<()> {
    let api_key = cfg.require_api_key()?;
    let chain_name = args.chain.as_str();
    let chain = resolve_chain(chain_name)?;
    let client = EtherscanClient::new(api_key, chain.chain_id);

    let cache_parts = ["tx", args.hash.as_str()];
    let txhash_params = format!(r#"{{"txhash":"{}"}}"#, args.hash);

    // Resolve (transaction, receipt, block timestamp) from cache or network.
    // A mined transaction is immutable, so it is cached permanently.
    let (tx, receipt, block_ts): (Value, Value, Option<u64>) = match client
        .cache_get(&cache_parts)
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
    {
        Some(cached) => (
            cached.get("transaction").cloned().unwrap_or(Value::Null),
            cached.get("receipt").cloned().unwrap_or(Value::Null),
            cached.get("timestamp").and_then(|t| t.as_u64()),
        ),
        None => {
            // tx and receipt are independent — fetch them concurrently.
            let (tx_res, receipt_res) = tokio::join!(
                client.proxy::<Value>("eth_getTransactionByHash", &txhash_params),
                client.proxy::<Value>("eth_getTransactionReceipt", &txhash_params),
            );
            let tx = tx_res?;
            if tx.is_null() {
                return Err(ScanevmError::NotFound(format!("transaction {}", args.hash)));
            }
            let receipt = receipt_res.unwrap_or(Value::Null);

            // Fetch the block timestamp only when the tx is mined.
            let block_ts = match tx["blockNumber"].as_str() {
                Some(block_hex) => {
                    let block_data: Value = client
                        .proxy(
                            "eth_getBlockByNumber",
                            &format!(r#"{{"tag":"{block_hex}","boolean":false}}"#),
                        )
                        .await
                        .unwrap_or(Value::Null);
                    block_data["timestamp"]
                        .as_str()
                        .and_then(|h| u64::from_str_radix(h.trim_start_matches("0x"), 16).ok())
                }
                None => None,
            };

            if !tx["blockNumber"].is_null() {
                let payload = serde_json::json!({
                    "transaction": tx,
                    "receipt": receipt,
                    "timestamp": block_ts,
                });
                if let Ok(s) = serde_json::to_string(&payload) {
                    client.cache_put(&cache_parts, &s, None);
                }
            }
            (tx, receipt, block_ts)
        }
    };

    let use_json = args.json || cfg.default_output == OutputFormat::Json;
    if use_json {
        let combined = serde_json::json!({
            "transaction": tx,
            "receipt": receipt,
        });
        print_json(&combined);
        return Ok(());
    }

    // Parse key fields
    let hash = tx["hash"].as_str().unwrap_or(&args.hash);
    let block = tx["blockNumber"]
        .as_str()
        .map(|s| {
            i64::from_str_radix(s.trim_start_matches("0x"), 16)
                .unwrap_or(0)
                .to_string()
        })
        .unwrap_or_else(|| "pending".to_string());
    let from = tx["from"].as_str().unwrap_or("-");
    let to = tx["to"].as_str().unwrap_or("(contract creation)");
    let value_hex = tx["value"].as_str().unwrap_or("0x0");
    let value_wei = u128::from_str_radix(value_hex.trim_start_matches("0x"), 16)
        .unwrap_or(0)
        .to_string();
    let gas_price_hex = tx["gasPrice"].as_str().unwrap_or("0x0");
    let gas_price_wei = u64::from_str_radix(gas_price_hex.trim_start_matches("0x"), 16)
        .unwrap_or(0)
        .to_string();
    let gas_limit_hex = tx["gas"].as_str().unwrap_or("0x0");
    let gas_limit = u64::from_str_radix(gas_limit_hex.trim_start_matches("0x"), 16).unwrap_or(0);
    let nonce_hex = tx["nonce"].as_str().unwrap_or("0x0");
    let nonce = u64::from_str_radix(nonce_hex.trim_start_matches("0x"), 16).unwrap_or(0);

    // Receipt fields
    let status = receipt["status"]
        .as_str()
        .map(|s| {
            if s == "0x1" {
                "Success ✓"
            } else {
                "Failed ✗"
            }
        })
        .unwrap_or("Unknown");
    let gas_used_hex = receipt["gasUsed"].as_str().unwrap_or("0x0");
    let gas_used = u64::from_str_radix(gas_used_hex.trim_start_matches("0x"), 16).unwrap_or(0);

    // Timestamp was resolved alongside the tx (from cache or the block fetch).
    let timestamp_row = block_ts
        .map(|ts| format_timestamp(&ts.to_string()))
        .unwrap_or_else(|| "pending".to_string());

    let input_data = tx["input"].as_str().unwrap_or("0x");
    let input_display = if input_data == "0x" || input_data.is_empty() {
        "0x (ETH transfer)".to_string()
    } else {
        format!("{} bytes", (input_data.len() - 2) / 2)
    };

    print_kv_table(&[
        ("Hash", hash.to_string()),
        ("Status", status.to_string()),
        ("Block", block.clone()),
        ("Timestamp", timestamp_row),
        ("From", from.to_string()),
        ("To", to.to_string()),
        ("Value", format_eth(&value_wei, chain.currency_symbol)),
        ("Gas Price", format_gwei(&gas_price_wei)),
        ("Gas Limit", gas_limit.to_string()),
        ("Gas Used", gas_used.to_string()),
        ("Nonce", nonce.to_string()),
        ("Input Data", input_display),
        ("Chain", chain.name.to_string()),
    ]);
    Ok(())
}
