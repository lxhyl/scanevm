mod cache;
mod chains;
mod client;
mod commands;
mod config;
mod diamond;
mod error;
mod output;
mod proxy;
mod types;

use clap::{Parser, Subcommand};
use colored::Colorize;
use config::Config;
use error::Result;

use commands::{
    balance::BalanceArgs, block::BlockArgs, chains::ChainsArgs, config::ConfigArgs,
    contract::ContractArgs, gas::GasArgs, token::TokenArgs, transfers::TransfersArgs, tx::TxArgs,
    txlist::TxlistArgs,
};

#[derive(Debug, Parser)]
#[command(
    name = "scanevm",
    about = "scanevm — fetch verified contract source and query EVM chains from the terminal",
    version,
    after_help = EXIT_CODES_HELP
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

/// Shown at the bottom of `--help`. Most commands accept `--json` for
/// machine-readable output; env vars: ETHERSCAN_API_KEY, SCANEVM_NO_CACHE.
const EXIT_CODES_HELP: &str = "\
Exit codes:
  0  success
  2  usage / config error (missing key, unknown chain, bad input)
  3  network error
  4  rate limited (retryable)
  5  invalid API key
  6  API error
  7  not found (no such block / tx)
  8  contract source not verified

Most commands accept --json. Env: ETHERSCAN_API_KEY, SCANEVM_NO_CACHE=1.";

#[derive(Debug, Subcommand)]
enum Commands {
    /// Get ETH balance for an address
    Balance(BalanceArgs),
    /// List transactions for an address
    Txlist(TxlistArgs),
    /// List ERC-20 / NFT token transfers
    Transfers(TransfersArgs),
    /// Contract source, ABI, bytecode, and proxy resolution
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
        std::process::exit(e.exit_code());
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
