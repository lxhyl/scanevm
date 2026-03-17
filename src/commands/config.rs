use clap::{Args, Subcommand};
use crate::config::{Config, OutputFormat};
use crate::error::Result;
use crate::output::{print_kv_table, success};
use std::str::FromStr;

#[derive(Debug, Args)]
pub struct ConfigArgs {
    #[command(subcommand)]
    pub subcommand: ConfigSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum ConfigSubcommand {
    /// Set the Etherscan API key
    SetKey {
        key: String,
    },
    /// Set the default output format (table|json)
    SetOutput {
        format: String,
    },
    /// Show current configuration
    Show,
}

pub async fn run(args: &ConfigArgs, cfg: &Config) -> Result<()> {
    match &args.subcommand {
        ConfigSubcommand::SetKey { key } => {
            let mut c = cfg.clone();
            c.api_key = Some(key.clone());
            c.save()?;
            println!("{}", success("API key saved."));
        }
        ConfigSubcommand::SetOutput { format } => {
            let fmt = OutputFormat::from_str(format)?;
            let mut c = cfg.clone();
            c.default_output = fmt;
            c.save()?;
            println!("{}", success(&format!("Default output set to '{format}'.")));
        }
        ConfigSubcommand::Show => {
            let key_display = cfg
                .api_key
                .as_deref()
                .map(|k| {
                    if k.len() > 8 {
                        format!("{}...{}", &k[..4], &k[k.len() - 4..])
                    } else {
                        "****".to_string()
                    }
                })
                .unwrap_or_else(|| "(not set)".to_string());

            let output_str = format!("{:?}", cfg.default_output).to_lowercase();

            print_kv_table(&[
                ("API Key", key_display),
                ("Default Output", output_str),
                ("Config Path", Config::path().display().to_string()),
            ]);
        }
    }
    Ok(())
}
