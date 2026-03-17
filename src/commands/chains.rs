use clap::Args;
use tabled::Tabled;
use crate::chains::CHAINS;
use crate::config::{Config, OutputFormat};
use crate::error::Result;
use crate::output::print_table;

#[derive(Debug, Args)]
pub struct ChainsArgs {
    /// Show testnets only
    #[arg(long)]
    pub testnets: bool,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Tabled, serde::Serialize)]
struct ChainRow {
    #[tabled(rename = "Name")]
    name: &'static str,
    #[tabled(rename = "Chain ID")]
    chain_id: u64,
    #[tabled(rename = "Symbol")]
    symbol: &'static str,
    #[tabled(rename = "Explorer")]
    explorer: &'static str,
    #[tabled(rename = "Testnet")]
    testnet: &'static str,
}

pub async fn run(args: &ChainsArgs, cfg: &Config) -> Result<()> {
    let rows: Vec<ChainRow> = CHAINS
        .iter()
        .filter(|c| !args.testnets || c.is_testnet)
        .map(|c| ChainRow {
            name: c.name,
            chain_id: c.chain_id,
            symbol: c.currency_symbol,
            explorer: c.explorer_url,
            testnet: if c.is_testnet { "yes" } else { "no" },
        })
        .collect();

    let use_json = args.json || cfg.default_output == OutputFormat::Json;
    if use_json {
        crate::output::print_json(&rows);
    } else {
        print_table(rows);
    }
    Ok(())
}
