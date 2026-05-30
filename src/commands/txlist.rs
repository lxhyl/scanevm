use crate::chains::resolve_chain;
use crate::client::EtherscanClient;
use crate::config::{Config, OutputFormat};
use crate::error::Result;
use crate::output::{
    format_eth, format_timestamp, print_json, print_table, truncate_addr, truncate_hash,
};
use crate::types::Transaction;
use clap::Args;
use tabled::Tabled;

#[derive(Debug, Args)]
pub struct TxlistArgs {
    /// Ethereum address
    #[arg(value_parser = crate::commands::parse_address)]
    pub address: String,

    /// Chain name or ID (e.g. ethereum, polygon, base, 137)
    #[arg(long, short = 'c')]
    pub chain: String,

    /// Max number of transactions (1-10000)
    #[arg(long, default_value = "20", value_parser = clap::value_parser!(u32).range(1..=10000))]
    pub limit: u32,

    /// Start block
    #[arg(long, default_value = "0")]
    pub start_block: u64,

    /// Sort order
    #[arg(long, default_value = "desc")]
    pub sort: crate::commands::SortOrder,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Tabled, serde::Serialize)]
struct TxRow {
    #[tabled(rename = "Hash")]
    hash: String,
    #[tabled(rename = "Block")]
    block: String,
    #[tabled(rename = "Time")]
    time: String,
    #[tabled(rename = "From")]
    from: String,
    #[tabled(rename = "To")]
    to: String,
    #[tabled(rename = "Value")]
    value: String,
    #[tabled(rename = "Status")]
    status: String,
}

pub async fn run(args: &TxlistArgs, cfg: &Config) -> Result<()> {
    let api_key = cfg.require_api_key()?;
    let chain_name = args.chain.as_str();
    let chain = resolve_chain(chain_name)?;
    let client = EtherscanClient::new(api_key, chain.chain_id);

    let limit = args.limit.to_string();
    let start_block = args.start_block.to_string();

    let txs: Vec<Transaction> = client
        .call(&[
            ("module", "account"),
            ("action", "txlist"),
            ("address", &args.address),
            ("startblock", &start_block),
            ("endblock", "99999999"),
            ("page", "1"),
            ("offset", &limit),
            ("sort", args.sort.as_str()),
        ])
        .await?;

    let use_json = args.json || cfg.default_output == OutputFormat::Json;
    if use_json {
        print_json(&txs);
        return Ok(());
    }

    let rows: Vec<TxRow> = txs
        .iter()
        .map(|tx| TxRow {
            hash: truncate_hash(&tx.hash),
            block: tx.block_number.clone(),
            time: format_timestamp(&tx.timestamp),
            from: truncate_addr(&tx.from),
            to: truncate_addr(&tx.to),
            value: format_eth(&tx.value, chain.currency_symbol),
            status: if tx.is_error == "0" {
                "✓".to_string()
            } else {
                "✗".to_string()
            },
        })
        .collect();

    print_table(rows);
    Ok(())
}
