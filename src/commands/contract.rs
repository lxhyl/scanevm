use crate::chains::{resolve_chain, ChainInfo};
use crate::client::EtherscanClient;
use crate::config::{Config, OutputFormat};
use crate::error::{Result, ScanevmError};
use crate::fetch::{disk_paths, report_json, Fetcher, Stored};
use crate::output::{print_json, print_kv_table};
use crate::rpc::Reader;
use crate::sources::lang_ext;
use crate::util::{run_limited, sanitize_rel_path, to_hex};
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
        #[arg(value_parser = crate::commands::parse_address)]
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
        /// Output metadata as JSON (ignored with --print/--save/--flatten)
        #[arg(long)]
        json: bool,
    },
    /// Fetch contract ABI
    Abi {
        /// Contract address
        #[arg(value_parser = crate::commands::parse_address)]
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
        #[arg(value_parser = crate::commands::parse_address)]
        address: String,
        /// Chain name or ID (e.g. ethereum, polygon, base, 137)
        #[arg(long, short = 'c')]
        chain: String,
        /// Output as JSON ({"address","bytecode"})
        #[arg(long)]
        json: bool,
    },
    /// Resolve the implementation address behind a proxy
    Impl {
        /// Proxy contract address
        #[arg(value_parser = crate::commands::parse_address)]
        address: String,
        /// Chain name or ID (e.g. ethereum, polygon, base, 137)
        #[arg(long, short = 'c')]
        chain: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// List or download a diamond / multi-facet proxy's facets
    Facets {
        /// Diamond contract address
        #[arg(value_parser = crate::commands::parse_address)]
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
        ContractSubcommand::Bytecode {
            address,
            chain,
            json,
        } => run_bytecode(address, chain, *json, cfg).await,
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

/// Facets fetched at once (as the scan core does).
const FACET_CONCURRENCY: usize = 4;

/// Fetch every facet's source, `FACET_CONCURRENCY` at a time, in facet order.
async fn fetch_facets(
    fetcher: &Fetcher<'_>,
    diamond: &crate::diamond::Facets,
) -> Vec<Result<Stored>> {
    let addrs: Vec<String> = diamond.facets.iter().map(|f| f.address.clone()).collect();
    run_limited(addrs, FACET_CONCURRENCY, |a| async move {
        fetcher.fetch(&a).await.into_source()
    })
    .await
}

/// The user's Etherscan client, when a key is configured. Without one,
/// scanevm uses the keyless providers and public RPCs.
fn etherscan_client(cfg: &Config, chain: &ChainInfo) -> Option<EtherscanClient> {
    let keys = cfg.keys();
    (!keys.is_empty()).then(|| EtherscanClient::new(keys, chain.chain_id, cfg.no_cache))
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
    let chain = resolve_chain(chain_name)?;
    let es = etherscan_client(cfg, &chain);
    let reader = Reader::new(chain, es.as_ref(), cfg.no_cache);
    let fetcher = Fetcher::new(chain, &reader, es.as_ref(), cfg.no_cache);

    let mut target = address.to_string();

    // Follow a proxy to its implementation if asked.
    if follow_impl {
        let found = crate::proxy::resolve_implementation(&reader, es.as_ref(), address).await?;
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
                if let Some(d) = crate::diamond::resolve_facets(&reader, address).await? {
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
        target = impl_addr;
    }

    let report = fetcher.fetch(&target).await;
    let use_json = use_json_flag || cfg.default_output == OutputFormat::Json;

    // Metadata mode: no --print, --save, or --flatten.
    if !do_print && save_dir.is_none() && !flatten {
        if use_json {
            // The report says what happened even without source; the exit code
            // still tells not verified (8) / no code (7) / errors apart.
            print_json(&report_json(&report));
            return report.into_source().map(|_| ());
        }
        let stored = report.into_source()?;
        print_metadata(&stored, &chain);
        return Ok(());
    }

    let stored = report.into_source()?;
    let f = &stored.fetched;

    if flatten {
        let flat = flatten_files(&f.sources);
        match save_dir {
            Some(dir) => {
                let name = if f.name.is_empty() {
                    "Contract"
                } else {
                    &f.name
                };
                save_file(
                    dir,
                    &format!("{}.flat.{}", sanitize_rel_path(name), lang_ext(&f.language)),
                    &flat,
                )?;
            }
            None => print!("{flat}"),
        }
        return Ok(());
    }

    let disk = disk_paths(f);
    for ((path, content), rel) in f.sources.iter().zip(&disk) {
        if let Some(dir) = save_dir {
            save_file(dir, rel, content)?;
        }
        if do_print {
            println!("// === {path} ===");
            println!("{content}");
            println!();
        }
    }
    Ok(())
}

fn print_metadata(stored: &Stored, chain: &ChainInfo) {
    let f = &stored.fetched;
    let mut rows: Vec<(&str, String)> = vec![("Contract", f.name.clone())];
    let compiler = if f.language == "solidity" || f.language == "unknown" {
        f.compiler.version.clone()
    } else {
        format!("{} ({})", f.compiler.version, f.language)
    };
    rows.push(("Compiler", compiler));
    if let Some(evm) = &f.compiler.evm_version {
        rows.push(("EVM Version", evm.clone()));
    }
    if let Some(o) = &f.compiler.optimizer {
        rows.push((
            "Optimization",
            match (o.enabled, o.runs) {
                (true, Some(runs)) => format!("Yes ({runs} runs)"),
                (true, None) => "Yes".to_string(),
                (false, _) => "No".to_string(),
            },
        ));
    }
    if let Some(l) = &f.license {
        rows.push(("License", l.clone()));
    }
    if let Some(h) = &f.proxy_hint {
        rows.push((
            "Proxy",
            if !h.is_proxy {
                "No".to_string()
            } else if h.implementations.is_empty() {
                "Yes".to_string()
            } else {
                format!("Yes (impl: {})", h.implementations.join(", "))
            },
        ));
    }
    if let Some(args) = &f.constructor_args {
        rows.push(("Constructor Args", truncate_hex(args)));
    }
    let source = match &f.similar_to {
        Some(twin) => format!("{} (similar to {twin})", f.provider.as_str()),
        None => format!("{} ({})", f.provider.as_str(), f.quality.as_str()),
    };
    rows.push(("Source", source));
    rows.push(("Files", f.sources.len().to_string()));
    rows.push(("Chain", chain.name.to_string()));
    print_kv_table(&rows);
}

fn truncate_hex(s: &str) -> String {
    if s.len() <= 42 {
        s.to_string()
    } else {
        format!("{}… ({} hex chars)", &s[..42], s.len())
    }
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
    // Paths are sanitized before they get here (see `util::sanitize_rel_path`);
    // refuse anything that would still escape the target directory.
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

/// The ABI of a fetched source set, compact JSON.
fn abi_of(stored: &Stored, address: &str) -> Result<String> {
    let f = &stored.fetched;
    let abi = f.abi.as_ref().ok_or_else(|| {
        ScanevmError::ApiError(format!(
            "{} returned source but no ABI for {address}",
            f.provider.as_str()
        ))
    })?;
    Ok(serde_json::to_string(abi)?)
}

async fn run_abi(
    address: &str,
    chain_name: &str,
    follow_impl: bool,
    use_json_flag: bool,
    cfg: &Config,
) -> Result<()> {
    let chain = resolve_chain(chain_name)?;
    let es = etherscan_client(cfg, &chain);
    let reader = Reader::new(chain, es.as_ref(), cfg.no_cache);
    let fetcher = Fetcher::new(chain, &reader, es.as_ref(), cfg.no_cache);

    let mut target = address.to_string();
    if follow_impl {
        match crate::proxy::resolve_implementation(&reader, es.as_ref(), address).await? {
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

    let stored = fetcher.fetch(&target).await.into_source()?;
    let abi = abi_of(&stored, &target)?;

    let use_json = use_json_flag || cfg.default_output == OutputFormat::Json;
    if use_json {
        // Compact JSON on one line, for piping.
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

fn looks_like_proxy_abi(abi: &str) -> bool {
    // Common proxy entrypoints that signal "this is the proxy, not the logic".
    abi.contains("\"upgradeTo\"")
        || abi.contains("\"upgradeToAndCall\"")
        || abi.contains("\"implementation\"") && abi.contains("\"admin\"")
}

async fn run_bytecode(
    address: &str,
    chain_name: &str,
    use_json_flag: bool,
    cfg: &Config,
) -> Result<()> {
    let chain = resolve_chain(chain_name)?;
    let es = etherscan_client(cfg, &chain);
    let reader = Reader::new(chain, es.as_ref(), cfg.no_cache);

    // Deployed bytecode is immutable; the reader caches it for good.
    let code = reader.code(address).await?;
    let bytecode = format!("0x{}", to_hex(&code));

    let use_json = use_json_flag || cfg.default_output == OutputFormat::Json;
    if use_json {
        print_json(&serde_json::json!({ "address": address, "bytecode": bytecode }));
    } else {
        println!("{bytecode}");
    }
    Ok(())
}

async fn run_impl(
    address: &str,
    chain_name: &str,
    use_json_flag: bool,
    cfg: &Config,
) -> Result<()> {
    let chain = resolve_chain(chain_name)?;
    let es = etherscan_client(cfg, &chain);
    let reader = Reader::new(chain, es.as_ref(), cfg.no_cache);
    let use_json = use_json_flag || cfg.default_output == OutputFormat::Json;

    let found = crate::proxy::resolve_implementation(&reader, es.as_ref(), address).await?;

    // A storage-slot hit is an unambiguous single-implementation proxy.
    if let Some(f) = &found {
        if f.via != "Etherscan" {
            return print_impl(address, f, chain.name, use_json);
        }
    }

    // Otherwise it might be a diamond — a single "implementation" would mislead.
    if let Some(d) = crate::diamond::resolve_facets(&reader, address).await? {
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
    let chain = resolve_chain(chain_name)?;
    let es = etherscan_client(cfg, &chain);
    let reader = Reader::new(chain, es.as_ref(), cfg.no_cache);
    let fetcher = Fetcher::new(chain, &reader, es.as_ref(), cfg.no_cache);

    let diamond = crate::diamond::resolve_facets(&reader, address)
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
        return save_facet_sources(&fetcher, &diamond, dir, flatten).await;
    }

    if with_abi {
        return merge_facet_abis(&fetcher, &diamond).await;
    }

    // Look up each facet's verified name for a friendlier listing.
    let named: Vec<(String, String, usize)> = diamond
        .facets
        .iter()
        .zip(fetch_facets(&fetcher, &diamond).await)
        .map(|(facet, src)| {
            let name = src
                .ok()
                .map(|s| s.fetched.name)
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| "(unverified)".to_string());
            (facet.address.clone(), name, facet.selectors.len())
        })
        .collect();

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
                    " (truncated: event logs ran past the page limit)"
                } else {
                    ""
                }
            )
            .dimmed()
        );
    }
    Ok(())
}

async fn merge_facet_abis(fetcher: &Fetcher<'_>, diamond: &crate::diamond::Facets) -> Result<()> {
    let mut merged: Vec<serde_json::Value> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut skipped = 0usize;
    for (facet, src) in diamond
        .facets
        .iter()
        .zip(fetch_facets(fetcher, diamond).await)
    {
        let abi = match src.and_then(|s| abi_of(&s, &facet.address)) {
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
    fetcher: &Fetcher<'_>,
    diamond: &crate::diamond::Facets,
    dir: &Path,
    flatten: bool,
) -> Result<()> {
    let mut saved = 0usize;
    let mut skipped = 0usize;
    for (facet, src) in diamond
        .facets
        .iter()
        .zip(fetch_facets(fetcher, diamond).await)
    {
        let stored = match src {
            Ok(s) => s,
            Err(e) => {
                eprintln!("{}", format!("skipped {} ({e})", facet.address).dimmed());
                skipped += 1;
                continue;
            }
        };
        let f = &stored.fetched;
        let label = sanitize_rel_path(if f.name.is_empty() {
            &facet.address
        } else {
            &f.name
        });
        if flatten {
            save_file(
                dir,
                &format!("{label}.flat.{}", lang_ext(&f.language)),
                &flatten_files(&f.sources),
            )?;
        } else {
            let facet_dir = dir.join(&label);
            for ((_, content), rel) in f.sources.iter().zip(disk_paths(f)) {
                save_file(&facet_dir, &rel, content)?;
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
            format!("note: {skipped} facet(s) had no verified source and were skipped").dimmed()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn save_refuses_escaping_paths() {
        let dir = std::env::temp_dir().join("scanevm-save-test");
        assert!(save_file(&dir, "../x.sol", "x").is_err());
        assert!(save_file(&dir, "/abs.sol", "x").is_err());
    }
}
