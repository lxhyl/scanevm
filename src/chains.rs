//! The chain table: names, explorers, which verified-source providers to ask
//! (in order), and public RPCs for reading chain state.
//!
//! Any numeric chain id works even if it isn't listed (see [`resolve_chain`]):
//! Sourcify, Etherscan v2 and Routescan all take a bare chain id. Listing a
//! chain adds names, a Blockscout instance, RPCs and a provider order.
//!
//! Provider and RPC facts were checked on 2026-11-25 (a verified contract on
//! each chain, `eth_chainId` + `eth_getCode` on each RPC). Etherscan v2 no
//! longer serves scroll, zksync, blast, fantom, cronos, moonriver or
//! blast-sepolia, so those use the other providers. The Blockscout instances
//! for polygon, arbitrum, base and gnosis sit behind a Cloudflare challenge for
//! API clients and are left out. Not reachable from the checking machine, so
//! unconfirmed: the blast and blast-sepolia RPCs.

use serde::Serialize;

use crate::error::{Result, ScanevmError};

/// A verified-source provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Sourcify,
    Etherscan,
    Blockscout,
    Routescan,
}

impl Provider {
    pub fn as_str(self) -> &'static str {
        match self {
            Provider::Sourcify => "sourcify",
            Provider::Etherscan => "etherscan",
            Provider::Blockscout => "blockscout",
            Provider::Routescan => "routescan",
        }
    }
}

use Provider::{Blockscout as BS, Etherscan as ES, Routescan as RS, Sourcify as SF};

/// Provider order for a chain id that isn't in the table.
pub const DEFAULT_SOURCES: &[Provider] = &[SF, ES, RS];

#[derive(Debug, Clone, Copy)]
pub struct ChainInfo {
    pub name: &'static str,
    /// Display name ("BSC", "HyperEVM").
    pub title: &'static str,
    pub aliases: &'static [&'static str],
    pub chain_id: u64,
    pub currency_symbol: &'static str,
    pub is_testnet: bool,
    /// Explorer base URL; empty when unknown.
    pub explorer_url: &'static str,
    /// Providers to ask, best first.
    pub sources: &'static [Provider],
    /// Blockscout instance base URL (needed for the Blockscout provider).
    pub blockscout: Option<&'static str>,
    /// Routescan network: "mainnet" or "testnet".
    pub routescan: Option<&'static str>,
    /// Public JSON-RPC endpoints, tried in order.
    pub rpc: &'static [&'static str],
    /// False for a bare chain id that isn't in the table.
    pub known: bool,
}

impl ChainInfo {
    /// `ethereum (1)`, or `chain 12345` for a chain not in the table.
    pub fn label(&self) -> String {
        if self.known {
            format!("{} ({})", self.name, self.chain_id)
        } else {
            format!("chain {}", self.chain_id)
        }
    }
}

macro_rules! chain {
    ($id:expr, $name:expr, $title:expr, [$($alias:expr),*], $sym:expr, $test:expr, $explorer:expr,
     [$($src:expr),*], $bs:expr, $rs:expr, [$($rpc:expr),*]) => {
        ChainInfo {
            name: $name,
            title: $title,
            aliases: &[$($alias),*],
            chain_id: $id,
            currency_symbol: $sym,
            is_testnet: $test,
            explorer_url: $explorer,
            sources: &[$($src),*],
            blockscout: $bs,
            routescan: $rs,
            rpc: &[$($rpc),*],
            known: true,
        }
    };
}

pub static CHAINS: &[ChainInfo] = &[
    // ---- mainnets ----
    chain!(
        1,
        "ethereum",
        "Ethereum",
        ["eth", "mainnet"],
        "ETH",
        false,
        "https://etherscan.io",
        [SF, ES, BS, RS],
        Some("https://eth.blockscout.com"),
        Some("mainnet"),
        [
            "https://eth.blockrazor.xyz",
            "https://ethereum-rpc.publicnode.com",
            "https://eth.drpc.org"
        ]
    ),
    chain!(
        137,
        "polygon",
        "Polygon",
        ["matic", "pol"],
        "POL",
        false,
        "https://polygonscan.com",
        [SF, ES],
        None,
        None,
        ["https://polygon-bor-rpc.publicnode.com"]
    ),
    chain!(
        56,
        "bsc",
        "BSC",
        ["bnb", "binance"],
        "BNB",
        false,
        "https://bscscan.com",
        [SF, ES],
        None,
        None,
        [
            "https://bsc-dataseed.bnbchain.org",
            "https://bsc-rpc.publicnode.com"
        ]
    ),
    chain!(
        42161,
        "arbitrum",
        "Arbitrum",
        ["arb", "arb1", "arbitrum-one"],
        "ETH",
        false,
        "https://arbiscan.io",
        [SF, ES],
        None,
        None,
        [
            "https://arb1.arbitrum.io/rpc",
            "https://arbitrum-one-rpc.publicnode.com"
        ]
    ),
    chain!(
        10,
        "optimism",
        "Optimism",
        ["op"],
        "ETH",
        false,
        "https://optimistic.etherscan.io",
        [SF, ES, BS],
        Some("https://explorer.optimism.io"),
        None,
        [
            "https://mainnet.optimism.io",
            "https://optimism-rpc.publicnode.com"
        ]
    ),
    chain!(
        8453,
        "base",
        "Base",
        [],
        "ETH",
        false,
        "https://basescan.org",
        [SF, ES],
        None,
        None,
        [
            "https://mainnet.base.org",
            "https://base-rpc.publicnode.com"
        ]
    ),
    chain!(
        43114,
        "avalanche",
        "Avalanche",
        ["avax", "avalanche-c"],
        "AVAX",
        false,
        "https://snowscan.xyz",
        [SF, ES, RS],
        None,
        Some("mainnet"),
        [
            "https://api.avax.network/ext/bc/C/rpc",
            "https://avalanche-c-chain-rpc.publicnode.com"
        ]
    ),
    chain!(
        59144,
        "linea",
        "Linea",
        [],
        "ETH",
        false,
        "https://lineascan.build",
        [SF, ES],
        None,
        None,
        [
            "https://rpc.linea.build",
            "https://linea-rpc.publicnode.com"
        ]
    ),
    chain!(
        534352,
        "scroll",
        "Scroll",
        [],
        "ETH",
        false,
        "https://scroll.blockscout.com",
        [SF, BS],
        Some("https://scroll.blockscout.com"),
        None,
        ["https://rpc.scroll.io", "https://scroll-rpc.publicnode.com"]
    ),
    chain!(
        324,
        "zksync",
        "zkSync Era",
        ["zks", "zksync-era"],
        "ETH",
        false,
        "https://explorer.zksync.io",
        [SF, BS],
        Some("https://zksync.blockscout.com"),
        None,
        ["https://mainnet.era.zksync.io", "https://zksync.drpc.org"]
    ),
    chain!(
        81457,
        "blast",
        "Blast",
        [],
        "ETH",
        false,
        "https://blastscan.io",
        [SF],
        None,
        None,
        ["https://rpc.blast.io"]
    ),
    chain!(
        42220,
        "celo",
        "Celo",
        [],
        "CELO",
        false,
        "https://celoscan.io",
        [SF, ES, BS],
        Some("https://celo.blockscout.com"),
        None,
        ["https://forno.celo.org", "https://celo-rpc.publicnode.com"]
    ),
    chain!(
        100,
        "gnosis",
        "Gnosis",
        ["xdai"],
        "xDAI",
        false,
        "https://gnosisscan.io",
        [SF, ES],
        None,
        None,
        [
            "https://rpc.gnosischain.com",
            "https://gnosis-rpc.publicnode.com"
        ]
    ),
    chain!(
        250,
        "fantom",
        "Fantom",
        ["ftm"],
        "FTM",
        false,
        "https://ftmscan.com",
        [SF],
        None,
        None,
        ["https://rpcapi.fantom.network", "https://fantom.drpc.org"]
    ),
    chain!(
        25,
        "cronos",
        "Cronos",
        ["cro"],
        "CRO",
        false,
        "https://explorer.cronos.org",
        [SF],
        None,
        None,
        [
            "https://evm.cronos.org",
            "https://cronos-evm-rpc.publicnode.com"
        ]
    ),
    chain!(
        1285,
        "moonriver",
        "Moonriver",
        ["movr"],
        "MOVR",
        false,
        "https://moonriver.moonscan.io",
        [SF],
        None,
        None,
        ["https://moonriver.drpc.org"]
    ),
    chain!(
        143,
        "monad",
        "Monad",
        [],
        "MON",
        false,
        "https://monadscan.com",
        [SF, ES],
        None,
        None,
        ["https://rpc.monad.xyz", "https://rpc1.monad.xyz"]
    ),
    chain!(
        146,
        "sonic",
        "Sonic",
        [],
        "S",
        false,
        "https://sonicscan.org",
        [SF, ES],
        None,
        None,
        ["https://rpc.soniclabs.com"]
    ),
    chain!(
        196,
        "xlayer",
        "X Layer",
        ["x-layer", "okx"],
        "OKB",
        false,
        "https://www.oklink.com/xlayer",
        [SF],
        None,
        None,
        ["https://xlayerrpc.okx.com"]
    ),
    chain!(
        999,
        "hyperevm",
        "HyperEVM",
        ["hyper", "hyperliquid", "hl"],
        "HYPE",
        false,
        "https://hyperevmscan.io",
        [SF, ES],
        None,
        None,
        [
            "https://hyperliquid.drpc.org",
            "https://rpc.hyperliquid.xyz/evm"
        ]
    ),
    chain!(
        5000,
        "mantle",
        "Mantle",
        [],
        "MNT",
        false,
        "https://mantlescan.xyz",
        [SF, ES, RS],
        None,
        Some("mainnet"),
        ["https://rpc.mantle.xyz"]
    ),
    chain!(
        9745,
        "plasma",
        "Plasma",
        [],
        "XPL",
        false,
        "https://plasmascan.to",
        [SF, ES],
        None,
        None,
        ["https://rpc.plasma.to"]
    ),
    chain!(
        80094,
        "berachain",
        "Berachain",
        ["bera"],
        "BERA",
        false,
        "https://berascan.com",
        [SF, ES],
        None,
        None,
        ["https://rpc.berachain.com"]
    ),
    // ---- testnets ----
    chain!(
        11155111,
        "sepolia",
        "Sepolia",
        ["eth-sepolia"],
        "ETH",
        true,
        "https://sepolia.etherscan.io",
        [SF, ES, BS, RS],
        Some("https://eth-sepolia.blockscout.com"),
        Some("testnet"),
        ["https://ethereum-sepolia-rpc.publicnode.com"]
    ),
    chain!(
        80002,
        "amoy",
        "Polygon Amoy",
        ["polygon-amoy"],
        "POL",
        true,
        "https://amoy.polygonscan.com",
        [SF, ES],
        None,
        None,
        ["https://polygon-amoy-bor-rpc.publicnode.com"]
    ),
    chain!(
        97,
        "bsc-testnet",
        "BSC Testnet",
        [],
        "tBNB",
        true,
        "https://testnet.bscscan.com",
        [SF, ES],
        None,
        None,
        [
            "https://data-seed-prebsc-1-s1.bnbchain.org:8545",
            "https://bsc-testnet-rpc.publicnode.com"
        ]
    ),
    chain!(
        421614,
        "arbitrum-sepolia",
        "Arbitrum Sepolia",
        ["arb-sepolia"],
        "ETH",
        true,
        "https://sepolia.arbiscan.io",
        [SF, ES],
        None,
        None,
        [
            "https://sepolia-rollup.arbitrum.io/rpc",
            "https://arbitrum-sepolia-rpc.publicnode.com"
        ]
    ),
    chain!(
        11155420,
        "optimism-sepolia",
        "OP Sepolia",
        ["op-sepolia"],
        "ETH",
        true,
        "https://sepolia-optimism.etherscan.io",
        [SF, ES, BS],
        Some("https://optimism-sepolia.blockscout.com"),
        None,
        [
            "https://sepolia.optimism.io",
            "https://optimism-sepolia-rpc.publicnode.com"
        ]
    ),
    chain!(
        84532,
        "base-sepolia",
        "Base Sepolia",
        [],
        "ETH",
        true,
        "https://sepolia.basescan.org",
        [SF, ES, BS],
        Some("https://base-sepolia.blockscout.com"),
        None,
        [
            "https://sepolia.base.org",
            "https://base-sepolia-rpc.publicnode.com"
        ]
    ),
    chain!(
        59141,
        "linea-sepolia",
        "Linea Sepolia",
        [],
        "ETH",
        true,
        "https://sepolia.lineascan.build",
        [SF, ES],
        None,
        None,
        [
            "https://rpc.sepolia.linea.build",
            "https://linea-sepolia-rpc.publicnode.com"
        ]
    ),
    chain!(
        168587773,
        "blast-sepolia",
        "Blast Sepolia",
        [],
        "ETH",
        true,
        "https://sepolia.blastscan.io",
        [RS],
        None,
        Some("testnet"),
        ["https://sepolia.blast.io"]
    ),
];

/// Resolve a chain name, alias, decimal id or `0x` hex id. A numeric id that
/// isn't in the table still works, with [`DEFAULT_SOURCES`] and no RPCs.
pub fn resolve_chain(input: &str) -> Result<ChainInfo> {
    let lower = input.trim().to_lowercase();
    let id = match lower.strip_prefix("0x") {
        Some(hex) => u64::from_str_radix(hex, 16).ok(),
        None => lower.parse::<u64>().ok(),
    };
    for chain in CHAINS {
        if chain.name == lower
            || chain.aliases.contains(&lower.as_str())
            || Some(chain.chain_id) == id
        {
            return Ok(*chain);
        }
    }
    match id {
        Some(id) if id > 0 => Ok(generic(id)),
        _ => Err(ScanevmError::UnknownChain(input.to_string())),
    }
}

/// An entry for a chain id the table doesn't list.
fn generic(id: u64) -> ChainInfo {
    // One small leak per run, so the entry has the table's `&'static` shape.
    let name: &'static str = Box::leak(id.to_string().into_boxed_str());
    ChainInfo {
        name,
        title: name,
        aliases: &[],
        chain_id: id,
        currency_symbol: "",
        is_testnet: false,
        explorer_url: "",
        sources: DEFAULT_SOURCES,
        blockscout: None,
        routescan: None,
        rpc: &[],
        known: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_by_name() {
        assert_eq!(resolve_chain("ethereum").unwrap().chain_id, 1);
        assert_eq!(resolve_chain("polygon").unwrap().chain_id, 137);
        assert_eq!(resolve_chain("HyperEVM").unwrap().chain_id, 999);
    }

    #[test]
    fn resolve_by_alias() {
        assert_eq!(resolve_chain("eth").unwrap().chain_id, 1);
        assert_eq!(resolve_chain("matic").unwrap().chain_id, 137);
        assert_eq!(resolve_chain("arb").unwrap().chain_id, 42161);
    }

    #[test]
    fn resolve_by_chain_id() {
        assert_eq!(resolve_chain("1").unwrap().name, "ethereum");
        assert_eq!(resolve_chain("8453").unwrap().name, "base");
        assert_eq!(resolve_chain("0x3e7").unwrap().name, "hyperevm");
    }

    #[test]
    fn unlisted_chain_id_gets_a_generic_entry() {
        let c = resolve_chain("7777777").unwrap();
        assert!(!c.known);
        assert_eq!(c.sources, DEFAULT_SOURCES);
        assert!(c.rpc.is_empty());
        assert_eq!(c.label(), "chain 7777777");
    }

    #[test]
    fn resolve_unknown() {
        assert!(resolve_chain("notachain").is_err());
        assert!(resolve_chain("0").is_err());
    }

    #[test]
    fn table_is_consistent() {
        let mut seen_ids = std::collections::HashSet::new();
        let mut seen_names = std::collections::HashSet::new();
        for c in CHAINS {
            assert!(seen_ids.insert(c.chain_id), "duplicate id {}", c.chain_id);
            for n in std::iter::once(&c.name).chain(c.aliases) {
                assert!(seen_names.insert(*n), "duplicate name {n}");
            }
            assert!(!c.sources.is_empty(), "{} has no sources", c.name);
            assert_eq!(
                c.sources.contains(&Provider::Blockscout),
                c.blockscout.is_some(),
                "{}: blockscout source and instance must go together",
                c.name
            );
            assert_eq!(
                c.sources.contains(&Provider::Routescan),
                c.routescan.is_some(),
                "{}: routescan source and network must go together",
                c.name
            );
            assert!(!c.rpc.is_empty(), "{} has no RPC", c.name);
        }
    }
}
