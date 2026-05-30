use crate::chains::resolve_chain;
use crate::client::EtherscanClient;
use crate::config::{Config, OutputFormat};
use crate::error::{Result, ScanevmError};
use crate::output::{print_json, print_kv_table};
use crate::types::ContractSource;
use clap::{Args, Subcommand};
use colored::Colorize;
use std::path::{Component, Path, PathBuf};

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
        /// Concatenate all source files into a single flattened output
        #[arg(long)]
        flatten: bool,
        /// If the address is a proxy, fetch the implementation's source instead
        #[arg(long = "impl")]
        follow_impl: bool,
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
        /// If the address is a proxy, fetch the implementation's ABI instead
        #[arg(long = "impl")]
        follow_impl: bool,
        /// Output as compact JSON (default pretty-prints)
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
    /// Resolve the implementation address behind a proxy
    Impl {
        /// Proxy contract address
        address: String,
        /// Chain name or ID (e.g. ethereum, polygon, base, 137)
        #[arg(long, short = 'c')]
        chain: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// List the facets of an EIP-2535 diamond
    Facets {
        /// Diamond contract address
        address: String,
        /// Chain name or ID (e.g. ethereum, polygon, base, 137)
        #[arg(long, short = 'c')]
        chain: String,
        /// Download every facet's source into this directory (one subdir per facet)
        #[arg(long)]
        save: Option<PathBuf>,
        /// With --save, write each facet flattened to a single <Name>.flat.sol
        #[arg(long)]
        flatten: bool,
        /// Fetch and merge every facet's ABI into one combined ABI
        #[arg(long)]
        abi: bool,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

pub async fn run(args: &ContractArgs, cfg: &Config) -> Result<()> {
    match &args.subcommand {
        ContractSubcommand::Source {
            address,
            chain,
            print,
            save,
            flatten,
            follow_impl,
            json,
        } => {
            run_source(
                address,
                chain,
                *print,
                save.as_ref(),
                *flatten,
                *follow_impl,
                *json,
                cfg,
            )
            .await
        }
        ContractSubcommand::Abi {
            address,
            chain,
            follow_impl,
            json,
        } => run_abi(address, chain, *follow_impl, *json, cfg).await,
        ContractSubcommand::Bytecode { address, chain } => run_bytecode(address, chain, cfg).await,
        ContractSubcommand::Impl {
            address,
            chain,
            json,
        } => run_impl(address, chain, *json, cfg).await,
        ContractSubcommand::Facets {
            address,
            chain,
            save,
            flatten,
            abi,
            json,
        } => run_facets(address, chain, save.as_ref(), *flatten, *abi, *json, cfg).await,
    }
}

async fn fetch_source(client: &EtherscanClient, address: &str) -> Result<ContractSource> {
    // Verified source is immutable, so cache it permanently.
    let sources: Vec<ContractSource> = client
        .call_cached(
            None,
            &[
                ("module", "contract"),
                ("action", "getsourcecode"),
                ("address", address),
            ],
        )
        .await?;
    Ok(sources.into_iter().next().unwrap_or_default())
}

#[allow(clippy::too_many_arguments)]
async fn run_source(
    address: &str,
    chain_name: &str,
    do_print: bool,
    save_dir: Option<&PathBuf>,
    flatten: bool,
    follow_impl: bool,
    use_json_flag: bool,
    cfg: &Config,
) -> Result<()> {
    let api_key = cfg.require_api_key()?;
    let chain = resolve_chain(chain_name)?;
    let client = EtherscanClient::new(api_key, chain.chain_id);

    let mut source = fetch_source(&client, address).await?;
    let mut display_address = address.to_string();

    // Follow a proxy to its implementation if asked.
    if follow_impl {
        let found = crate::proxy::resolve_implementation(&client, address).await?;
        // A storage-slot hit is an unambiguous single-implementation proxy.
        let slot_impl = found
            .as_ref()
            .filter(|f| f.via != "Etherscan")
            .map(|f| f.address.clone());

        let impl_addr = match slot_impl {
            Some(addr) => addr,
            None => {
                // Etherscan-flagged or unknown — a diamond would have many facets,
                // so following a single "implementation" would be misleading.
                if let Some(d) = crate::diamond::resolve_facets(&client, address).await? {
                    eprintln!(
                        "{}",
                        format!(
                            "{address} is a multi-facet proxy (diamond) with {} facets — \
                             run: scanevm contract facets {address} --save <dir>",
                            d.facets.len()
                        )
                        .yellow()
                    );
                    return Ok(());
                }
                match found {
                    Some(f) => f.address,
                    None => {
                        return Err(ScanevmError::ApiError(format!(
                            "{address} does not appear to be a proxy (no implementation found)"
                        )));
                    }
                }
            }
        };
        eprintln!(
            "{}",
            format!("→ following proxy {address} to implementation {impl_addr}").dimmed()
        );
        source = fetch_source(&client, &impl_addr).await?;
        display_address = impl_addr;
    }

    // An unverified contract comes back with empty source/name — report it
    // clearly (exit code 8) instead of printing a blank metadata table.
    if source.source_code.is_empty() && source.contract_name.is_empty() {
        return Err(ScanevmError::NotVerified(display_address));
    }

    let use_json = use_json_flag || cfg.default_output == OutputFormat::Json;

    // Metadata mode: no --print, --save, or --flatten.
    if !do_print && save_dir.is_none() && !flatten {
        if use_json {
            print_json(&source);
        } else {
            print_metadata(&source, chain.name);
        }
        return Ok(());
    }

    let files = parse_source_files(&source.source_code, &source.contract_name);

    if flatten {
        let flat = flatten_files(&files);
        match save_dir {
            Some(dir) => {
                let name = if source.contract_name.is_empty() {
                    "Contract"
                } else {
                    &source.contract_name
                };
                save_file(dir, &format!("{name}.flat.sol"), &flat)?;
            }
            None => print!("{flat}"),
        }
        return Ok(());
    }

    for (path, content) in &files {
        if let Some(dir) = save_dir {
            save_file(dir, path, content)?;
        }
        if do_print {
            println!("// === {path} ===");
            println!("{content}");
            println!();
        }
    }
    Ok(())
}

fn print_metadata(source: &ContractSource, chain_name: &str) {
    let proxy_info = if source.proxy == "1" {
        format!(
            "Yes (impl: {})",
            source.implementation.as_deref().unwrap_or("unknown")
        )
    } else {
        "No".to_string()
    };

    let mut rows: Vec<(&str, String)> = vec![
        ("Contract", source.contract_name.clone()),
        ("Compiler", source.compiler_version.clone()),
    ];
    if !source.evm_version.is_empty() && source.evm_version != "Default" {
        rows.push(("EVM Version", source.evm_version.clone()));
    }
    rows.push((
        "Optimization",
        if source.optimization_used == "1" {
            format!("Yes ({} runs)", source.runs)
        } else {
            "No".to_string()
        },
    ));
    rows.push(("License", source.license_type.clone()));
    rows.push(("Proxy", proxy_info));
    if !source.constructor_arguments.is_empty() {
        rows.push((
            "Constructor Args",
            truncate_hex(&source.constructor_arguments),
        ));
    }
    rows.push(("Chain", chain_name.to_string()));
    print_kv_table(&rows);
}

fn truncate_hex(s: &str) -> String {
    if s.len() <= 42 {
        s.to_string()
    } else {
        format!("{}… ({} hex chars)", &s[..42], s.len())
    }
}

/// Parse the `SourceCode` field into `(path, content)` pairs.
///
/// Handles Etherscan's three shapes: a standard-JSON-input object wrapped in
/// double braces (`{{ "language": .., "sources": { path: { content } } }}`),
/// a bare double-brace map (`{{ path: { content } }}`), and a plain single
/// Solidity file. Always returns at least one file.
fn parse_source_files(raw: &str, contract_name: &str) -> Vec<(String, String)> {
    if raw.starts_with("{{") && raw.ends_with("}}") {
        // Strip one layer of braces to get valid JSON.
        let inner = &raw[1..raw.len() - 1];
        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(inner) {
            let sources_map = parsed
                .get("sources")
                .cloned()
                .unwrap_or_else(|| parsed.clone());
            if let Some(map) = sources_map.as_object() {
                let mut files: Vec<(String, String)> = map
                    .iter()
                    .map(|(path, obj)| {
                        let content = obj
                            .get("content")
                            .and_then(|c| c.as_str())
                            .unwrap_or("")
                            .to_string();
                        (path.clone(), content)
                    })
                    .collect();
                files.sort_by(|a, b| a.0.cmp(&b.0));
                if !files.is_empty() {
                    return files;
                }
            }
        }
    }

    let name = if contract_name.is_empty() {
        "Contract"
    } else {
        contract_name
    };
    vec![(format!("{name}.sol"), raw.to_string())]
}

fn flatten_files(files: &[(String, String)]) -> String {
    let mut out = String::new();
    for (path, content) in files {
        out.push_str(&format!("// === {path} ===\n"));
        out.push_str(content);
        if !content.ends_with('\n') {
            out.push('\n');
        }
        out.push('\n');
    }
    out
}

fn save_file(base_dir: &Path, relative_path: &str, content: &str) -> Result<()> {
    // The relative path comes from the API response, so reject anything that
    // would escape the target directory (absolute paths, `..`, drive prefixes).
    let candidate = Path::new(relative_path);
    let safe = candidate
        .components()
        .all(|c| matches!(c, Component::Normal(_)));
    if !safe {
        return Err(ScanevmError::Config(format!(
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
    follow_impl: bool,
    use_json_flag: bool,
    cfg: &Config,
) -> Result<()> {
    let api_key = cfg.require_api_key()?;
    let chain = resolve_chain(chain_name)?;
    let client = EtherscanClient::new(api_key, chain.chain_id);

    let mut target = address.to_string();
    if follow_impl {
        match crate::proxy::resolve_implementation(&client, address).await? {
            Some(found) => {
                eprintln!(
                    "{}",
                    format!("→ proxy {address} implementation {}", found.address).dimmed()
                );
                target = found.address;
            }
            None => {
                return Err(ScanevmError::ApiError(format!(
                    "{address} does not appear to be a proxy (no implementation found)"
                )));
            }
        }
    }

    let abi: String = client
        .call_cached(
            None,
            &[
                ("module", "contract"),
                ("action", "getabi"),
                ("address", &target),
            ],
        )
        .await
        .map_err(|e| not_verified_if_applicable(e, &target))?;

    let use_json = use_json_flag || cfg.default_output == OutputFormat::Json;
    if use_json {
        // The ABI is already a compact JSON string — emit it verbatim for piping.
        println!("{abi}");
    } else if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&abi) {
        println!("{}", serde_json::to_string_pretty(&parsed)?);
    } else {
        println!("{abi}");
    }

    // If the user did not follow the proxy and this looks like a proxy's own
    // ABI, nudge them — the proxy ABI usually can't call the real contract.
    if !follow_impl && looks_like_proxy_abi(&abi) {
        eprintln!(
            "{}",
            "note: this looks like a proxy ABI; re-run with --impl to fetch the implementation's ABI"
                .dimmed()
        );
    }
    Ok(())
}

fn not_verified_if_applicable(e: ScanevmError, address: &str) -> ScanevmError {
    match e {
        ScanevmError::ApiError(msg) if msg.to_lowercase().contains("not verified") => {
            ScanevmError::NotVerified(address.to_string())
        }
        other => other,
    }
}

fn looks_like_proxy_abi(abi: &str) -> bool {
    // Common proxy entrypoints that signal "this is the proxy, not the logic".
    abi.contains("\"upgradeTo\"")
        || abi.contains("\"upgradeToAndCall\"")
        || abi.contains("\"implementation\"") && abi.contains("\"admin\"")
}

async fn run_bytecode(address: &str, chain_name: &str, cfg: &Config) -> Result<()> {
    let api_key = cfg.require_api_key()?;
    let chain = resolve_chain(chain_name)?;
    let client = EtherscanClient::new(api_key, chain.chain_id);

    // Deployed bytecode at a given address is immutable; cache permanently.
    let bytecode: String = client
        .proxy_cached(
            None,
            "eth_getCode",
            &format!(r#"{{"address":"{address}","tag":"latest"}}"#),
        )
        .await?;

    println!("{bytecode}");
    Ok(())
}

async fn run_impl(
    address: &str,
    chain_name: &str,
    use_json_flag: bool,
    cfg: &Config,
) -> Result<()> {
    let api_key = cfg.require_api_key()?;
    let chain = resolve_chain(chain_name)?;
    let client = EtherscanClient::new(api_key, chain.chain_id);
    let use_json = use_json_flag || cfg.default_output == OutputFormat::Json;

    let found = crate::proxy::resolve_implementation(&client, address).await?;

    // A storage-slot hit is an unambiguous single-implementation proxy.
    if let Some(f) = &found {
        if f.via != "Etherscan" {
            return print_impl(address, f, chain.name, use_json);
        }
    }

    // Otherwise it might be a diamond — a single "implementation" would mislead.
    if let Some(d) = crate::diamond::resolve_facets(&client, address).await? {
        let n = d.facets.len();
        eprintln!(
            "{}",
            format!("{address} is a multi-facet proxy (diamond) with {n} facets — run: scanevm contract facets {address}")
                .yellow()
        );
        if use_json {
            print_json(&serde_json::json!({
                "address": address,
                "type": "diamond",
                "facet_count": n,
                "chain": chain.name,
            }));
        }
        return Ok(());
    }

    match found {
        Some(f) => print_impl(address, &f, chain.name, use_json),
        None => Err(ScanevmError::ApiError(format!(
            "{address} does not appear to be a proxy \
             (no implementation found via EIP-1967/1822/beacon, diamond, or Etherscan)"
        ))),
    }
}

fn print_impl(
    address: &str,
    found: &crate::proxy::Implementation,
    chain_name: &str,
    use_json: bool,
) -> Result<()> {
    if use_json {
        print_json(&serde_json::json!({
            "proxy": address,
            "implementation": found.address,
            "via": found.via,
            "chain": chain_name,
        }));
    } else {
        print_kv_table(&[
            ("Proxy", address.to_string()),
            ("Implementation", found.address.clone()),
            ("Detected via", found.via.to_string()),
            ("Chain", chain_name.to_string()),
        ]);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn run_facets(
    address: &str,
    chain_name: &str,
    save_dir: Option<&PathBuf>,
    flatten: bool,
    with_abi: bool,
    use_json_flag: bool,
    cfg: &Config,
) -> Result<()> {
    let api_key = cfg.require_api_key()?;
    let chain = resolve_chain(chain_name)?;
    let client = EtherscanClient::new(api_key, chain.chain_id);

    let diamond = crate::diamond::resolve_facets(&client, address)
        .await?
        .ok_or_else(|| {
            ScanevmError::ApiError(format!(
                "{address} is not a recognized diamond / multi-facet proxy \
                 (no facets found via DiamondLoupe, DiamondCut, or SelectorToFacetSet logs)"
            ))
        })?;

    let source_label = match diamond.source {
        crate::diamond::Source::Loupe => "DiamondLoupe facets()",
        crate::diamond::Source::LoupeAddressesOnly => "DiamondLoupe facetAddresses()",
        crate::diamond::Source::DiamondCutLogs => "DiamondCut event logs",
        crate::diamond::Source::SelectorEventLogs => "SelectorToFacetSet event logs",
    };

    if let Some(dir) = save_dir {
        return save_facet_sources(&client, &diamond, dir, flatten).await;
    }

    if with_abi {
        return merge_facet_abis(&client, &diamond).await;
    }

    // Look up each facet's verified name (cached) for a friendlier listing.
    let mut named: Vec<(String, String, usize)> = Vec::with_capacity(diamond.facets.len());
    for facet in &diamond.facets {
        let name = fetch_source(&client, &facet.address)
            .await
            .ok()
            .map(|s| s.contract_name)
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| "(unverified)".to_string());
        named.push((facet.address.clone(), name, facet.selectors.len()));
    }

    let use_json = use_json_flag || cfg.default_output == OutputFormat::Json;
    if use_json {
        print_json(&serde_json::json!({
            "diamond": address,
            "source": source_label,
            "truncated": diamond.truncated,
            "facet_count": diamond.facets.len(),
            "facets": diamond.facets.iter().zip(&named).map(|(f, n)| serde_json::json!({
                "address": f.address,
                "name": n.1,
                "selectors": f.selectors,
            })).collect::<Vec<_>>(),
            "chain": chain.name,
        }));
    } else {
        #[derive(tabled::Tabled)]
        struct FacetRow {
            #[tabled(rename = "Facet")]
            address: String,
            #[tabled(rename = "Name")]
            name: String,
            #[tabled(rename = "Selectors")]
            selectors: String,
        }
        let rows: Vec<FacetRow> = named
            .into_iter()
            .map(|(address, name, count)| FacetRow {
                address,
                name,
                selectors: if count == 0 {
                    "?".to_string()
                } else {
                    count.to_string()
                },
            })
            .collect();
        crate::output::print_table(rows);
        eprintln!(
            "{}",
            format!(
                "{} facets via {source_label}{}",
                diamond.facets.len(),
                if diamond.truncated {
                    " (truncated: getLogs hit its 1000-record cap)"
                } else {
                    ""
                }
            )
            .dimmed()
        );
    }
    Ok(())
}

async fn merge_facet_abis(
    client: &EtherscanClient,
    diamond: &crate::diamond::Facets,
) -> Result<()> {
    let mut merged: Vec<serde_json::Value> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut skipped = 0usize;
    for facet in &diamond.facets {
        let abi: String = match client
            .call_cached(
                None,
                &[
                    ("module", "contract"),
                    ("action", "getabi"),
                    ("address", &facet.address),
                ],
            )
            .await
        {
            Ok(a) => a,
            Err(_) => {
                skipped += 1;
                continue;
            }
        };
        if let Ok(serde_json::Value::Array(items)) = serde_json::from_str::<serde_json::Value>(&abi)
        {
            for item in items {
                // Dedupe identical entries shared across facets.
                let key = item.to_string();
                if seen.insert(key) {
                    merged.push(item);
                }
            }
        }
    }
    println!("{}", serde_json::to_string_pretty(&merged)?);
    if skipped > 0 {
        eprintln!(
            "{}",
            format!("note: {skipped} facet(s) had no verified ABI and were skipped").dimmed()
        );
    }
    Ok(())
}

/// Download every facet's verified source into `dir`. Without `--flatten` each
/// facet gets its own subdirectory (named after the facet contract); with it,
/// each facet is written as a single `<Name>.flat.sol`.
async fn save_facet_sources(
    client: &EtherscanClient,
    diamond: &crate::diamond::Facets,
    dir: &Path,
    flatten: bool,
) -> Result<()> {
    let mut saved = 0usize;
    let mut skipped = 0usize;
    for facet in &diamond.facets {
        let source = match fetch_source(client, &facet.address).await {
            Ok(s) if !s.source_code.is_empty() => s,
            _ => {
                eprintln!(
                    "{}",
                    format!("skipped {} (source not verified)", facet.address).dimmed()
                );
                skipped += 1;
                continue;
            }
        };
        let label = if source.contract_name.is_empty() {
            facet.address.clone()
        } else {
            source.contract_name.clone()
        };
        let files = parse_source_files(&source.source_code, &source.contract_name);
        if flatten {
            save_file(dir, &format!("{label}.flat.sol"), &flatten_files(&files))?;
        } else {
            let facet_dir = dir.join(&label);
            for (path, content) in &files {
                save_file(&facet_dir, path, content)?;
            }
        }
        saved += 1;
    }
    println!(
        "{}",
        format!("Saved {saved} facet(s) to {}", dir.display()).green()
    );
    if skipped > 0 {
        eprintln!(
            "{}",
            format!("note: {skipped} facet(s) had unverified source and were skipped").dimmed()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_single_file_solidity() {
        let files = parse_source_files("contract Foo {}", "Foo");
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].0, "Foo.sol");
        assert_eq!(files[0].1, "contract Foo {}");
    }

    #[test]
    fn parses_standard_json_input() {
        let raw = r#"{{"language":"Solidity","sources":{"a/B.sol":{"content":"X"},"c/D.sol":{"content":"Y"}}}}"#;
        let files = parse_source_files(raw, "B");
        assert_eq!(files.len(), 2);
        // sorted by path
        assert_eq!(files[0], ("a/B.sol".to_string(), "X".to_string()));
        assert_eq!(files[1], ("c/D.sol".to_string(), "Y".to_string()));
    }

    #[test]
    fn parses_bare_double_brace_map() {
        let raw = r#"{{"Only.sol":{"content":"Z"}}}"#;
        let files = parse_source_files(raw, "Only");
        assert_eq!(files, vec![("Only.sol".to_string(), "Z".to_string())]);
    }

    #[test]
    fn malformed_double_brace_falls_back_to_single_file() {
        let raw = "{{not json}}";
        let files = parse_source_files(raw, "Fallback");
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].0, "Fallback.sol");
    }

    #[test]
    fn empty_contract_name_uses_default() {
        let files = parse_source_files("pragma solidity ^0.8;", "");
        assert_eq!(files[0].0, "Contract.sol");
    }

    #[test]
    fn flatten_joins_with_headers() {
        let files = vec![
            ("a.sol".to_string(), "AAA".to_string()),
            ("b.sol".to_string(), "BBB\n".to_string()),
        ];
        let flat = flatten_files(&files);
        assert!(flat.contains("// === a.sol ==="));
        assert!(flat.contains("// === b.sol ==="));
        assert!(flat.contains("AAA"));
        assert!(flat.contains("BBB"));
    }

    #[test]
    fn proxy_abi_is_detected() {
        assert!(looks_like_proxy_abi(
            r#"[{"name":"upgradeTo","type":"function"}]"#
        ));
        assert!(!looks_like_proxy_abi(
            r#"[{"name":"transfer","type":"function"}]"#
        ));
    }

    #[test]
    fn not_verified_mapping() {
        let mapped = not_verified_if_applicable(
            ScanevmError::ApiError("Contract source code not verified".to_string()),
            "0xabc",
        );
        assert!(matches!(mapped, ScanevmError::NotVerified(_)));
        let untouched =
            not_verified_if_applicable(ScanevmError::ApiError("other".to_string()), "0xabc");
        assert!(matches!(untouched, ScanevmError::ApiError(_)));
    }
}
