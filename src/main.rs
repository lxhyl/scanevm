mod error;
mod config;
mod chains;
mod client;
mod types;
mod output;
mod commands;

use clap::{Parser, Subcommand};
use colored::Colorize;
use config::Config;
use error::Result;

use commands::{
    balance::BalanceArgs,
    txlist::TxlistArgs,
    transfers::TransfersArgs,
    contract::ContractArgs,
    gas::GasArgs,
    token::TokenArgs,
    tx::TxArgs,
    block::BlockArgs,
    chains::ChainsArgs,
    config::ConfigArgs,
};

#[derive(Debug, Parser)]
#[command(
    name = "etherscan",
    about = "Etherscan CLI — query blockchain data from the terminal",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Get ETH balance for an address
    Balance(BalanceArgs),
    /// List transactions for an address
    Txlist(TxlistArgs),
    /// List ERC-20 / NFT token transfers
    Transfers(TransfersArgs),
    /// Contract source, ABI, and bytecode
    Contract(ContractArgs),
    /// Current gas prices
    Gas(GasArgs),
    /// ERC-20 token information
    Token(TokenArgs),
    /// Transaction details by hash
    Tx(TxArgs),
    /// Block details by number
    Block(BlockArgs),
    /// List supported chains
    Chains(ChainsArgs),
    /// Configure API key and defaults
    Config(ConfigArgs),
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let cli = Cli::parse();
    let cfg = Config::load();
    if let Err(e) = run(cli.command, &cfg).await {
        eprintln!("{} {}", "error:".red().bold(), e);
        std::process::exit(1);
    }
}

async fn run(command: Commands, cfg: &Config) -> Result<()> {
    match command {
        Commands::Balance(args) => commands::balance::run(&args, cfg).await,
        Commands::Txlist(args) => commands::txlist::run(&args, cfg).await,
        Commands::Transfers(args) => commands::transfers::run(&args, cfg).await,
        Commands::Contract(args) => commands::contract::run(&args, cfg).await,
        Commands::Gas(args) => commands::gas::run(&args, cfg).await,
        Commands::Token(args) => commands::token::run(&args, cfg).await,
        Commands::Tx(args) => commands::tx::run(&args, cfg).await,
        Commands::Block(args) => commands::block::run(&args, cfg).await,
        Commands::Chains(args) => commands::chains::run(&args, cfg).await,
        Commands::Config(args) => commands::config::run(&args, cfg).await,
    }
}
