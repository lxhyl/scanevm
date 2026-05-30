use crate::chains::resolve_chain;
use crate::client::EtherscanClient;
use crate::config::{Config, OutputFormat};
use crate::error::Result;
use crate::output::{format_eth, print_json, print_kv_table};
use clap::Args;

#[derive(Debug, Args)]
pub struct BalanceArgs {
    /// Ethereum address
    pub address: String,

    /// Chain name or ID (e.g. ethereum, polygon, base, 137)
    #[arg(long, short = 'c')]
    pub chain: String,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

pub async fn run(args: &BalanceArgs, cfg: &Config) -> Result<()> {
    let api_key = cfg.require_api_key()?;
    let chain_name = args.chain.as_str();
    let chain = resolve_chain(chain_name)?;
    let client = EtherscanClient::new(api_key, chain.chain_id);

    let balance: String = client
        .call_cached(
            Some(std::time::Duration::from_secs(30)),
            &[
                ("module", "account"),
                ("action", "balance"),
                ("address", &args.address),
                ("tag", "latest"),
            ],
        )
        .await?;

    let use_json = args.json || cfg.default_output == OutputFormat::Json;
    if use_json {
        print_json(&serde_json::json!({
            "address": args.address,
            "balance_wei": balance,
            "balance": format_eth(&balance, chain.currency_symbol),
            "chain": chain.name,
        }));
    } else {
        print_kv_table(&[
            ("Address", args.address.clone()),
            ("Balance", format_eth(&balance, chain.currency_symbol)),
            ("Chain", chain.name.to_string()),
        ]);
    }
    Ok(())
}
