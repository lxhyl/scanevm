use crate::chains::resolve_chain;
use crate::client::EtherscanClient;
use crate::config::{Config, OutputFormat};
use crate::error::{Result, ScanevmError};
use crate::output::{format_timestamp, print_json, print_kv_table};
use clap::Args;

#[derive(Debug, Args)]
pub struct BlockArgs {
    /// Block number (or "latest")
    pub number: String,

    /// Chain name or ID (e.g. ethereum, polygon, base, 137)
    #[arg(long, short = 'c')]
    pub chain: String,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

pub async fn run(args: &BlockArgs, cfg: &Config) -> Result<()> {
    let api_key = cfg.require_api_key()?;
    let chain_name = args.chain.as_str();
    let chain = resolve_chain(chain_name)?;
    let client = EtherscanClient::new(api_key, chain.chain_id);

    let is_latest = args.number.eq_ignore_ascii_case("latest");
    let tag = if is_latest {
        "latest".to_string()
    } else {
        format!("0x{:x}", parse_block_number(&args.number)?)
    };

    let params = format!(r#"{{"tag":"{tag}","boolean":false}}"#);
    // A specific block is immutable once mined, so cache it permanently;
    // "latest" must always hit the network.
    let block: serde_json::Value = if is_latest {
        client.proxy("eth_getBlockByNumber", &params).await?
    } else {
        client
            .proxy_cached(None, "eth_getBlockByNumber", &params)
            .await?
    };

    if block.is_null() {
        return Err(ScanevmError::NotFound(format!("block {}", args.number)));
    }

    let use_json = args.json || cfg.default_output == OutputFormat::Json;
    if use_json {
        print_json(&block);
        return Ok(());
    }

    let number_hex = block["number"].as_str().unwrap_or("0x0");
    let number = u64::from_str_radix(number_hex.trim_start_matches("0x"), 16).unwrap_or(0);
    let ts_hex = block["timestamp"].as_str().unwrap_or("0x0");
    let ts = u64::from_str_radix(ts_hex.trim_start_matches("0x"), 16).unwrap_or(0);
    let gas_used_hex = block["gasUsed"].as_str().unwrap_or("0x0");
    let gas_used = u64::from_str_radix(gas_used_hex.trim_start_matches("0x"), 16).unwrap_or(0);
    let gas_limit_hex = block["gasLimit"].as_str().unwrap_or("0x0");
    let gas_limit = u64::from_str_radix(gas_limit_hex.trim_start_matches("0x"), 16).unwrap_or(0);
    let base_fee_hex = block["baseFeePerGas"].as_str().unwrap_or("0x0");
    let base_fee = u64::from_str_radix(base_fee_hex.trim_start_matches("0x"), 16).unwrap_or(0);

    let tx_count = block["transactions"]
        .as_array()
        .map(|a| a.len())
        .unwrap_or(0);

    let miner = block["miner"].as_str().unwrap_or("-");
    let hash = block["hash"].as_str().unwrap_or("-");

    let utilization = if gas_limit > 0 {
        format!("{:.1}%", gas_used as f64 / gas_limit as f64 * 100.0)
    } else {
        "-".to_string()
    };

    print_kv_table(&[
        ("Block Number", number.to_string()),
        ("Hash", hash.to_string()),
        ("Timestamp", format_timestamp(&ts.to_string())),
        ("Miner", miner.to_string()),
        ("Transactions", tx_count.to_string()),
        ("Gas Used", gas_used.to_string()),
        ("Gas Limit", gas_limit.to_string()),
        ("Utilization", utilization),
        ("Base Fee", format!("{:.2} Gwei", base_fee as f64 / 1e9)),
        ("Chain", chain.name.to_string()),
    ]);
    Ok(())
}

/// Parse a block number given as decimal (`19000000`) or hex (`0x1212d20`).
/// Rejects anything else instead of silently querying block 0.
fn parse_block_number(s: &str) -> Result<u64> {
    let trimmed = s.trim();
    let parsed = match trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
    {
        Some(hex) => u64::from_str_radix(hex, 16),
        None => trimmed.parse::<u64>(),
    };
    parsed.map_err(|_| {
        ScanevmError::BadInput(format!(
            "invalid block number: '{s}' (expected a number or 'latest')"
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_decimal_and_hex() {
        assert_eq!(parse_block_number("19000000").unwrap(), 19_000_000);
        assert_eq!(parse_block_number("0x1212d20").unwrap(), 0x1212d20);
        assert_eq!(parse_block_number("  42 ").unwrap(), 42);
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_block_number("abc").is_err());
        assert!(parse_block_number("-1").is_err());
        assert!(parse_block_number("").is_err());
    }
}
