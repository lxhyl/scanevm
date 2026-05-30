use crate::chains::resolve_chain;
use crate::client::EtherscanClient;
use crate::config::{Config, OutputFormat};
use crate::error::Result;
use crate::output::{print_json, print_kv_table};
use crate::types::GasOracle;
use clap::Args;

#[derive(Debug, Args)]
pub struct GasArgs {
    /// Chain name or ID (e.g. ethereum, polygon, base, 137)
    #[arg(long, short = 'c')]
    pub chain: String,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

pub async fn run(args: &GasArgs, cfg: &Config) -> Result<()> {
    let api_key = cfg.require_api_key()?;
    let chain_name = args.chain.as_str();
    let chain = resolve_chain(chain_name)?;
    let client = EtherscanClient::new(api_key, chain.chain_id);

    let gas: GasOracle = client
        .call(&[("module", "gastracker"), ("action", "gasoracle")])
        .await?;

    let use_json = args.json || cfg.default_output == OutputFormat::Json;
    if use_json {
        print_json(&gas);
    } else {
        print_kv_table(&[
            ("Last Block", gas.last_block.clone()),
            ("Base Fee", format!("{} Gwei", gas.suggest_base_fee)),
            ("Safe (slow)", format!("{} Gwei", gas.safe_gas_price)),
            ("Standard", format!("{} Gwei", gas.propose_gas_price)),
            ("Fast", format!("{} Gwei", gas.fast_gas_price)),
            ("Gas Used Ratio", gas.gas_used_ratio.clone()),
            ("Chain", chain.name.to_string()),
        ]);
    }
    Ok(())
}
