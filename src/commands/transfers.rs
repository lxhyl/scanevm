use clap::Args;
use tabled::Tabled;
use crate::chains::resolve_chain;
use crate::client::EtherscanClient;
use crate::config::{Config, OutputFormat};
use crate::error::Result;
use crate::output::{format_token_amount, format_timestamp, print_json, print_table, truncate_hash, truncate_addr};
use crate::types::{NftTransfer, TokenTransfer};

#[derive(Debug, Args)]
pub struct TransfersArgs {
    /// Ethereum address
    pub address: String,

    /// Chain name or ID (e.g. ethereum, polygon, base, 137)
    #[arg(long, short = 'c')]
    pub chain: String,

    /// Show NFT transfers instead of ERC-20
    #[arg(long)]
    pub nft: bool,

    /// Max number of transfers
    #[arg(long, default_value = "20")]
    pub limit: u32,

    /// Sort order: asc or desc
    #[arg(long, default_value = "desc")]
    pub sort: String,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Tabled, serde::Serialize)]
struct Erc20Row {
    #[tabled(rename = "Hash")]
    hash: String,
    #[tabled(rename = "Time")]
    time: String,
    #[tabled(rename = "From")]
    from: String,
    #[tabled(rename = "To")]
    to: String,
    #[tabled(rename = "Token")]
    token: String,
    #[tabled(rename = "Amount")]
    amount: String,
}

#[derive(Tabled, serde::Serialize)]
struct NftRow {
    #[tabled(rename = "Hash")]
    hash: String,
    #[tabled(rename = "Time")]
    time: String,
    #[tabled(rename = "From")]
    from: String,
    #[tabled(rename = "To")]
    to: String,
    #[tabled(rename = "Collection")]
    collection: String,
    #[tabled(rename = "Token ID")]
    token_id: String,
}

pub async fn run(args: &TransfersArgs, cfg: &Config) -> Result<()> {
    let api_key = cfg.require_api_key()?;
    let chain_name = args.chain.as_str();
    let chain = resolve_chain(chain_name)?;
    let client = EtherscanClient::new(api_key, chain.chain_id);

    let limit = args.limit.to_string();
    let use_json = args.json || cfg.default_output == OutputFormat::Json;

    if args.nft {
        let transfers: Vec<NftTransfer> = client
            .call(&[
                ("module", "account"),
                ("action", "tokennfttx"),
                ("address", &args.address),
                ("page", "1"),
                ("offset", &limit),
                ("sort", &args.sort),
            ])
            .await?;

        if use_json {
            print_json(&transfers);
            return Ok(());
        }

        let rows: Vec<NftRow> = transfers
            .iter()
            .map(|t| NftRow {
                hash: truncate_hash(&t.hash),
                time: format_timestamp(&t.timestamp),
                from: truncate_addr(&t.from),
                to: truncate_addr(&t.to),
                collection: format!("{} ({})", t.token_name, t.token_symbol),
                token_id: t.token_id.clone(),
            })
            .collect();

        print_table(rows);
    } else {
        let transfers: Vec<TokenTransfer> = client
            .call(&[
                ("module", "account"),
                ("action", "tokentx"),
                ("address", &args.address),
                ("page", "1"),
                ("offset", &limit),
                ("sort", &args.sort),
            ])
            .await?;

        if use_json {
            print_json(&transfers);
            return Ok(());
        }

        let rows: Vec<Erc20Row> = transfers
            .iter()
            .map(|t| Erc20Row {
                hash: truncate_hash(&t.hash),
                time: format_timestamp(&t.timestamp),
                from: truncate_addr(&t.from),
                to: truncate_addr(&t.to),
                token: format!("{} ({})", t.token_name, t.token_symbol),
                amount: format!(
                    "{} {}",
                    format_token_amount(&t.value, &t.token_decimal),
                    t.token_symbol
                ),
            })
            .collect();

        print_table(rows);
    }
    Ok(())
}
