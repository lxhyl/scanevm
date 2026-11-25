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

use commands::{chains::ChainsArgs, config::ConfigArgs, contract::ContractArgs};

#[derive(Debug, Parser)]
#[command(
    name = "scanevm",
    about = "scanevm — fetch verified contract source, ABI and bytecode across EVM chains",
    version,
    after_help = EXIT_CODES_HELP
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Bypass the local response cache for this run (skip both read and write).
    /// Note: upgradeable (proxy) contracts are never cached regardless.
    #[arg(long, global = true)]
    no_cache: bool,
}

/// Shown at the bottom of `--help`. Most commands accept `--json` for
/// machine-readable output; env vars: ETHERSCAN_API_KEYS, SCANEVM_NO_CACHE.
const EXIT_CODES_HELP: &str = "\
Exit codes:
  0  success
  2  usage / config error (missing key, unknown chain, bad input)
  3  network error
  4  rate limited (retryable)
  5  invalid API key
  6  API error
  8  contract source not verified

Most commands accept --json. Configure multiple API keys (config set-key K1 K2 ...)
to pool past the 5 req/s per-key limit. Verified non-proxy source is cached
permanently; proxy (upgradeable) contracts are always fetched fresh. Pass
--no-cache to skip the cache entirely for one run.
Env: ETHERSCAN_API_KEYS (comma-separated pool), ETHERSCAN_API_KEY, SCANEVM_NO_CACHE=1.";

#[derive(Debug, Subcommand)]
enum Commands {
    /// Contract source, ABI, bytecode, and proxy resolution
    Contract(ContractArgs),
    /// List supported chains
    Chains(ChainsArgs),
    /// Configure API key and defaults
    Config(ConfigArgs),
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let cli = Cli::parse();
    let mut cfg = Config::load();
    cfg.no_cache = cli.no_cache;
    if let Err(e) = run(cli.command, &cfg).await {
        eprintln!("{} {}", "error:".red().bold(), e);
        std::process::exit(e.exit_code());
    }
}

async fn run(command: Commands, cfg: &Config) -> Result<()> {
    match command {
        Commands::Contract(args) => commands::contract::run(&args, cfg).await,
        Commands::Chains(args) => commands::chains::run(&args, cfg).await,
        Commands::Config(args) => commands::config::run(&args, cfg).await,
    }
}
