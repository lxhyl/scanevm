use crate::chains::{ChainInfo, Provider, CHAINS};
use crate::config::{Config, OutputFormat};
use crate::error::Result;
use crate::output::print_table;
use clap::Args;
use serde::Serialize;
use tabled::Tabled;

#[derive(Debug, Args)]
pub struct ChainsArgs {
    /// Show testnets only
    #[arg(long)]
    pub testnets: bool,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Tabled)]
struct ChainRow {
    #[tabled(rename = "Name")]
    name: &'static str,
    #[tabled(rename = "Chain ID")]
    chain_id: u64,
    #[tabled(rename = "Symbol")]
    symbol: &'static str,
    #[tabled(rename = "Sources")]
    sources: String,
    #[tabled(rename = "Explorer")]
    explorer: &'static str,
    #[tabled(rename = "Testnet")]
    testnet: &'static str,
}

/// One chain as `--json` prints it (the same fields as the scan service's
/// `/v1/chains`, so tools can read either).
#[derive(Serialize)]
struct ChainJson {
    id: u64,
    name: &'static str,
    title: &'static str,
    aliases: &'static [&'static str],
    testnet: bool,
    explorer: &'static str,
    sources: &'static [Provider],
}

pub async fn run(args: &ChainsArgs, cfg: &Config) -> Result<()> {
    let chains: Vec<&ChainInfo> = CHAINS
        .iter()
        .filter(|c| !args.testnets || c.is_testnet)
        .collect();

    let use_json = args.json || cfg.default_output == OutputFormat::Json;
    if use_json {
        let rows: Vec<ChainJson> = chains
            .iter()
            .map(|c| ChainJson {
                id: c.chain_id,
                name: c.name,
                title: c.title,
                aliases: c.aliases,
                testnet: c.is_testnet,
                explorer: c.explorer_url,
                sources: c.sources,
            })
            .collect();
        #[derive(Serialize)]
        struct Out {
            chains: Vec<ChainJson>,
        }
        crate::output::print_json(&Out { chains: rows });
    } else {
        let rows: Vec<ChainRow> = chains
            .iter()
            .map(|c| ChainRow {
                name: c.name,
                chain_id: c.chain_id,
                symbol: c.currency_symbol,
                sources: c
                    .sources
                    .iter()
                    .map(|p| p.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
                explorer: c.explorer_url,
                testnet: if c.is_testnet { "yes" } else { "no" },
            })
            .collect();
        print_table(rows);
        eprintln!(
            "{}",
            colored::Colorize::dimmed(
                "Any other numeric chain id also works (asks sourcify, etherscan, routescan)."
            )
        );
    }
    Ok(())
}
