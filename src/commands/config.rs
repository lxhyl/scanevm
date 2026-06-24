use crate::config::{split_keys, Config, OutputFormat};
use crate::error::{Result, ScanevmError};
use crate::output::{print_json, print_kv_table, success};
use clap::{Args, Subcommand};
use std::str::FromStr;

#[derive(Debug, Args)]
pub struct ConfigArgs {
    #[command(subcommand)]
    pub subcommand: ConfigSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum ConfigSubcommand {
    /// Set the Etherscan API key pool (replaces any existing keys).
    ///
    /// Pass several keys to raise throughput — each key is rate-limited
    /// separately (5 req/s on the free tier), so N keys sustain ~N× that.
    /// Keys may be space- or comma-separated.
    SetKey {
        #[arg(required = true, num_args = 1.., value_name = "KEY")]
        keys: Vec<String>,
    },
    /// Add one API key to the pool
    AddKey {
        #[arg(value_name = "KEY")]
        key: String,
    },
    /// Remove one API key from the pool
    RemoveKey {
        #[arg(value_name = "KEY")]
        key: String,
    },
    /// Set the default output format (table|json)
    SetOutput { format: String },
    /// Show current configuration
    Show {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

pub async fn run(args: &ConfigArgs, cfg: &Config) -> Result<()> {
    match &args.subcommand {
        ConfigSubcommand::SetKey { keys } => {
            // Accept both `set-key a b c` and `set-key "a,b,c"`.
            let parsed: Vec<String> = keys.iter().flat_map(|k| split_keys(k)).collect();
            if parsed.is_empty() {
                return Err(ScanevmError::BadInput("no API key provided".to_string()));
            }
            // Mutate the on-disk config, never the env-merged view, so a
            // transient ETHERSCAN_API_KEY is never persisted.
            let mut c = Config::from_file();
            c.api_key = None;
            c.api_keys = parsed;
            c.save()?;
            let n = c.keys().len();
            println!("{}", success(&format!("Saved {n} API key(s).")));
        }
        ConfigSubcommand::AddKey { key } => {
            let parsed = split_keys(key);
            if parsed.is_empty() {
                return Err(ScanevmError::BadInput("no API key provided".to_string()));
            }
            let mut c = Config::from_file();
            let mut list = c.keys();
            let added: Vec<String> = parsed.into_iter().filter(|k| !list.contains(k)).collect();
            if added.is_empty() {
                println!(
                    "{}",
                    success(&format!(
                        "Key already in the pool ({} key(s) total).",
                        list.len()
                    ))
                );
                return Ok(());
            }
            list.extend(added.iter().cloned());
            c.api_key = None;
            c.api_keys = list;
            c.save()?;
            let n = c.keys().len();
            println!(
                "{}",
                success(&format!(
                    "Added {} API key(s). Pool now has {n} key(s).",
                    added.len()
                ))
            );
        }
        ConfigSubcommand::RemoveKey { key } => {
            let mut c = Config::from_file();
            let mut list = c.keys();
            let before = list.len();
            list.retain(|k| k != key);
            if list.len() == before {
                return Err(ScanevmError::BadInput(format!(
                    "key not found in the pool ({before} key(s) configured)"
                )));
            }
            c.api_key = None;
            c.api_keys = list;
            c.save()?;
            let n = c.keys().len();
            println!(
                "{}",
                success(&format!("Removed API key. Pool now has {n} key(s)."))
            );
        }
        ConfigSubcommand::SetOutput { format } => {
            let fmt = OutputFormat::from_str(format)?;
            let mut c = Config::from_file();
            c.default_output = fmt;
            c.save()?;
            println!("{}", success(&format!("Default output set to '{format}'.")));
        }
        ConfigSubcommand::Show { json } => {
            let keys = cfg.keys();
            let previews: Vec<String> = keys.iter().map(|k| mask_key(k)).collect();
            let count = keys.len();

            let output_str = format!("{:?}", cfg.default_output).to_lowercase();
            let path = Config::path().display().to_string();

            let use_json = *json || cfg.default_output == OutputFormat::Json;
            if use_json {
                // Never emit full keys — only how many are set and masked previews.
                print_json(&serde_json::json!({
                    "api_key_set": count > 0,
                    "api_key_count": count,
                    "api_keys_preview": previews,
                    "default_output": output_str,
                    "config_path": path,
                }));
            } else {
                let keys_display = if count == 0 {
                    "(not set)".to_string()
                } else {
                    format!("{count} ({})", previews.join(", "))
                };
                print_kv_table(&[
                    ("API Keys", keys_display),
                    ("Default Output", output_str),
                    ("Config Path", path),
                ]);
            }
        }
    }
    Ok(())
}

/// Mask a key for display — first/last 4 chars, or `****` if too short to
/// partially reveal without exposing most of it.
fn mask_key(key: &str) -> String {
    if key.len() > 8 {
        format!("{}...{}", &key[..4], &key[key.len() - 4..])
    } else {
        "****".to_string()
    }
}
