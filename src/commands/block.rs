use clap::Args;
use crate::chains::resolve_chain;
use crate::client::EtherscanClient;
use crate::config::{Config, OutputFormat};
use crate::error::Result;
use crate::output::{format_timestamp, print_json, print_kv_table};

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

    let tag = if args.number.eq_ignore_ascii_case("latest") {
        "latest".to_string()
    } else {
        let n: u64 = args.number.parse().unwrap_or(0);
        format!("0x{:x}", n)
    };

    let block: serde_json::Value = client
        .proxy(
            "eth_getBlockByNumber",
            &format!(r#"{{"tag":"{}","boolean":false}}"#, tag),
        )
        .await?;

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
