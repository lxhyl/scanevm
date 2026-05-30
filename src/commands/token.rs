use crate::chains::resolve_chain;
use crate::client::EtherscanClient;
use crate::config::{Config, OutputFormat};
use crate::error::Result;
use crate::output::{format_token_amount, print_json, print_kv_table};
use crate::types::TokenInfo;
use clap::Args;

#[derive(Debug, Args)]
pub struct TokenArgs {
    /// Token contract address
    #[arg(value_parser = crate::commands::parse_address)]
    pub address: String,

    /// Chain name or ID (e.g. ethereum, polygon, base, 137)
    #[arg(long, short = 'c')]
    pub chain: String,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

pub async fn run(args: &TokenArgs, cfg: &Config) -> Result<()> {
    let api_key = cfg.require_api_key()?;
    let chain_name = args.chain.as_str();
    let chain = resolve_chain(chain_name)?;
    let client = EtherscanClient::new(api_key, chain.chain_id);

    // Fetch token info (metadata changes rarely — cache for an hour).
    let infos: Vec<TokenInfo> = client
        .call_cached(
            Some(std::time::Duration::from_secs(3600)),
            &[
                ("module", "token"),
                ("action", "tokeninfo"),
                ("contractaddress", &args.address),
            ],
        )
        .await?;

    // Fetch circulating supply (changes over time — short TTL).
    let supply: String = client
        .call_cached(
            Some(std::time::Duration::from_secs(60)),
            &[
                ("module", "stats"),
                ("action", "tokensupply"),
                ("contractaddress", &args.address),
            ],
        )
        .await
        .unwrap_or_else(|_| "0".to_string());

    let use_json = args.json || cfg.default_output == OutputFormat::Json;

    if let Some(info) = infos.first() {
        if use_json {
            print_json(&serde_json::json!({
                "info": info,
                "total_supply": supply,
            }));
        } else {
            let supply_fmt = format_token_amount(&supply, &info.divisor);
            print_kv_table(&[
                ("Contract", info.contract_address.clone()),
                ("Name", info.token_name.clone()),
                ("Symbol", info.symbol.clone()),
                ("Type", info.token_type.clone()),
                ("Decimals", info.divisor.clone()),
                ("Total Supply", format!("{} {}", supply_fmt, info.symbol)),
                ("Holders", info.holder_count.clone()),
                ("Verified", info.blue_checkmark.clone()),
                ("Website", info.website.clone()),
                ("Chain", chain.name.to_string()),
            ]);
            if !info.description.is_empty() {
                println!("\nDescription: {}", info.description);
            }
        }
    } else {
        eprintln!("No token info found for {}", args.address);
    }
    Ok(())
}
