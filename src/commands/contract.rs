use clap::{Args, Subcommand};
use std::path::{Component, Path, PathBuf};
use crate::chains::resolve_chain;
use crate::client::EtherscanClient;
use crate::config::{Config, OutputFormat};
use crate::error::{AppError, Result};
use crate::output::{print_json, print_kv_table};
use crate::types::ContractSource;

#[derive(Debug, Args)]
pub struct ContractArgs {
    #[command(subcommand)]
    pub subcommand: ContractSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum ContractSubcommand {
    /// Fetch and display verified contract source code
    Source {
        /// Contract address
        address: String,
        /// Chain name or ID (e.g. ethereum, polygon, base, 137)
        #[arg(long, short = 'c')]
        chain: String,
        /// Print source to stdout
        #[arg(long)]
        print: bool,
        /// Save source files to directory
        #[arg(long)]
        save: Option<PathBuf>,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Fetch contract ABI
    Abi {
        /// Contract address
        address: String,
        /// Chain name or ID (e.g. ethereum, polygon, base, 137)
        #[arg(long, short = 'c')]
        chain: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Fetch contract bytecode
    Bytecode {
        /// Contract address
        address: String,
        /// Chain name or ID (e.g. ethereum, polygon, base, 137)
        #[arg(long, short = 'c')]
        chain: String,
    },
}

pub async fn run(args: &ContractArgs, cfg: &Config) -> Result<()> {
    match &args.subcommand {
        ContractSubcommand::Source { address, chain, print, save, json } => {
            run_source(address, chain, *print, save.as_ref(), *json, cfg).await
        }
        ContractSubcommand::Abi { address, chain, json } => {
            run_abi(address, chain, *json, cfg).await
        }
        ContractSubcommand::Bytecode { address, chain } => {
            run_bytecode(address, chain, cfg).await
        }
    }
}

async fn run_source(
    address: &str,
    chain_name: &str,
    do_print: bool,
    save_dir: Option<&PathBuf>,
    use_json_flag: bool,
    cfg: &Config,
) -> Result<()> {
    let api_key = cfg.require_api_key()?;
    let chain = resolve_chain(chain_name)?;
    let client = EtherscanClient::new(api_key, chain.chain_id);

    let sources: Vec<ContractSource> = client
        .call(&[
            ("module", "contract"),
            ("action", "getsourcecode"),
            ("address", address),
        ])
        .await?;

    let use_json = use_json_flag || cfg.default_output == OutputFormat::Json;

    let source = sources.into_iter().next().unwrap_or_else(|| ContractSource {
        source_code: String::new(),
        abi: String::new(),
        contract_name: String::new(),
        compiler_version: String::new(),
        optimization_used: String::new(),
        runs: String::new(),
        license_type: String::new(),
        proxy: String::new(),
        implementation: None,
    });

    if use_json && !do_print && save_dir.is_none() {
        print_json(&source);
        return Ok(());
    }

    // Show metadata table unless just printing source
    if !do_print && save_dir.is_none() {
        let proxy_info = if source.proxy == "1" {
            format!(
                "Yes (impl: {})",
                source.implementation.as_deref().unwrap_or("unknown")
            )
        } else {
            "No".to_string()
        };

        print_kv_table(&[
            ("Contract", source.contract_name.clone()),
            ("Compiler", source.compiler_version.clone()),
            ("Optimization", if source.optimization_used == "1" {
                format!("Yes ({} runs)", source.runs)
            } else {
                "No".to_string()
            }),
            ("License", source.license_type.clone()),
            ("Proxy", proxy_info),
            ("Chain", chain.name.to_string()),
        ]);
        return Ok(());
    }

    // Detect Hardhat multi-file format: source_code starts with {{
    let raw = &source.source_code;
    if raw.starts_with("{{") && raw.ends_with("}}") {
        // Strip outer braces to get valid JSON
        let inner = &raw[1..raw.len() - 1];
        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(inner) {
            // Could be { language, sources: { "path": { content: "..." } } } (standard input JSON)
            // or directly { "path": { content: "..." } }
            let sources_map = parsed
                .get("sources")
                .or(Some(&parsed))
                .cloned()
                .unwrap_or(serde_json::Value::Null);

            if let Some(map) = sources_map.as_object() {
                for (file_path, file_obj) in map {
                    let content = file_obj["content"].as_str().unwrap_or("");
                    if let Some(dir) = save_dir {
                        save_file(dir, file_path, content)?;
                    }
                    if do_print {
                        println!("// === {} ===", file_path);
                        println!("{}", content);
                        println!();
                    }
                }
                return Ok(());
            }
        }
    }

    // Single-file source
    if let Some(dir) = save_dir {
        let filename = format!("{}.sol", source.contract_name);
        save_file(dir, &filename, raw)?;
        println!("Saved to {}/{}", dir.display(), filename);
    }
    if do_print {
        println!("{}", raw);
    }
    Ok(())
}

fn save_file(base_dir: &Path, relative_path: &str, content: &str) -> Result<()> {
    // The relative path comes from the API response, so reject anything that
    // would escape the target directory (absolute paths, `..`, drive prefixes).
    let candidate = Path::new(relative_path);
    let safe = candidate.components().all(|c| matches!(c, Component::Normal(_)));
    if !safe {
        return Err(AppError::Config(format!(
            "refusing to write unsafe source path: '{relative_path}'"
        )));
    }

    let full_path = base_dir.join(candidate);
    if let Some(parent) = full_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&full_path, content)?;
    println!("Saved: {}", full_path.display());
    Ok(())
}

async fn run_abi(
    address: &str,
    chain_name: &str,
    _use_json_flag: bool,
    cfg: &Config,
) -> Result<()> {
    let api_key = cfg.require_api_key()?;
    let chain = resolve_chain(chain_name)?;
    let client = EtherscanClient::new(api_key, chain.chain_id);

    let abi: String = client
        .call(&[
            ("module", "contract"),
            ("action", "getabi"),
            ("address", address),
        ])
        .await?;

    // Pretty-print JSON if possible
    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&abi) {
        println!("{}", serde_json::to_string_pretty(&parsed)?);
    } else {
        println!("{}", abi);
    }
    Ok(())
}

async fn run_bytecode(
    address: &str,
    chain_name: &str,
    cfg: &Config,
) -> Result<()> {
    let api_key = cfg.require_api_key()?;
    let chain = resolve_chain(chain_name)?;
    let client = EtherscanClient::new(api_key, chain.chain_id);

    let bytecode: String = client
        .proxy(
            "eth_getCode",
            &format!(r#"{{"address":"{}","tag":"latest"}}"#, address),
        )
        .await?;

    println!("{}", bytecode);
    Ok(())
}
