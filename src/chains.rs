use crate::error::{Result, ScanevmError};

pub struct ChainInfo {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub chain_id: u64,
    pub currency_symbol: &'static str,
    pub is_testnet: bool,
    pub explorer_url: &'static str,
}

pub static CHAINS: &[ChainInfo] = &[
    ChainInfo {
        name: "ethereum",
        aliases: &["eth", "mainnet", "1"],
        chain_id: 1,
        currency_symbol: "ETH",
        is_testnet: false,
        explorer_url: "https://etherscan.io",
    },
    ChainInfo {
        name: "polygon",
        aliases: &["matic", "137"],
        chain_id: 137,
        currency_symbol: "MATIC",
        is_testnet: false,
        explorer_url: "https://polygonscan.com",
    },
    ChainInfo {
        name: "bsc",
        aliases: &["bnb", "binance", "56"],
        chain_id: 56,
        currency_symbol: "BNB",
        is_testnet: false,
        explorer_url: "https://bscscan.com",
    },
    ChainInfo {
        name: "arbitrum",
        aliases: &["arb", "42161"],
        chain_id: 42161,
        currency_symbol: "ETH",
        is_testnet: false,
        explorer_url: "https://arbiscan.io",
    },
    ChainInfo {
        name: "optimism",
        aliases: &["op", "10"],
        chain_id: 10,
        currency_symbol: "ETH",
        is_testnet: false,
        explorer_url: "https://optimistic.etherscan.io",
    },
    ChainInfo {
        name: "base",
        aliases: &["8453"],
        chain_id: 8453,
        currency_symbol: "ETH",
        is_testnet: false,
        explorer_url: "https://basescan.org",
    },
    ChainInfo {
        name: "avalanche",
        aliases: &["avax", "43114"],
        chain_id: 43114,
        currency_symbol: "AVAX",
        is_testnet: false,
        explorer_url: "https://snowtrace.io",
    },
    ChainInfo {
        name: "linea",
        aliases: &["59144"],
        chain_id: 59144,
        currency_symbol: "ETH",
        is_testnet: false,
        explorer_url: "https://lineascan.build",
    },
    ChainInfo {
        name: "scroll",
        aliases: &["534352"],
        chain_id: 534352,
        currency_symbol: "ETH",
        is_testnet: false,
        explorer_url: "https://scrollscan.com",
    },
    ChainInfo {
        name: "zksync",
        aliases: &["zks", "324"],
        chain_id: 324,
        currency_symbol: "ETH",
        is_testnet: false,
        explorer_url: "https://era.zksync.network",
    },
    ChainInfo {
        name: "blast",
        aliases: &["81457"],
        chain_id: 81457,
        currency_symbol: "ETH",
        is_testnet: false,
        explorer_url: "https://blastscan.io",
    },
    ChainInfo {
        name: "celo",
        aliases: &["42220"],
        chain_id: 42220,
        currency_symbol: "CELO",
        is_testnet: false,
        explorer_url: "https://celoscan.io",
    },
    ChainInfo {
        name: "gnosis",
        aliases: &["xdai", "100"],
        chain_id: 100,
        currency_symbol: "xDAI",
        is_testnet: false,
        explorer_url: "https://gnosisscan.io",
    },
    ChainInfo {
        name: "fantom",
        aliases: &["ftm", "250"],
        chain_id: 250,
        currency_symbol: "FTM",
        is_testnet: false,
        explorer_url: "https://ftmscan.com",
    },
    ChainInfo {
        name: "cronos",
        aliases: &["cro", "25"],
        chain_id: 25,
        currency_symbol: "CRO",
        is_testnet: false,
        explorer_url: "https://cronoscan.com",
    },
    ChainInfo {
        name: "moonbeam",
        aliases: &["glmr", "1284"],
        chain_id: 1284,
        currency_symbol: "GLMR",
        is_testnet: false,
        explorer_url: "https://moonscan.io",
    },
    ChainInfo {
        name: "moonriver",
        aliases: &["movr", "1285"],
        chain_id: 1285,
        currency_symbol: "MOVR",
        is_testnet: false,
        explorer_url: "https://moonriver.moonscan.io",
    },
    // Testnets
    ChainInfo {
        name: "sepolia",
        aliases: &["11155111"],
        chain_id: 11155111,
        currency_symbol: "ETH",
        is_testnet: true,
        explorer_url: "https://sepolia.etherscan.io",
    },
    ChainInfo {
        name: "holesky",
        aliases: &["17000"],
        chain_id: 17000,
        currency_symbol: "ETH",
        is_testnet: true,
        explorer_url: "https://holesky.etherscan.io",
    },
    ChainInfo {
        name: "amoy",
        aliases: &["polygon-amoy", "80002"],
        chain_id: 80002,
        currency_symbol: "MATIC",
        is_testnet: true,
        explorer_url: "https://amoy.polygonscan.com",
    },
    ChainInfo {
        name: "bsc-testnet",
        aliases: &["97"],
        chain_id: 97,
        currency_symbol: "BNB",
        is_testnet: true,
        explorer_url: "https://testnet.bscscan.com",
    },
    ChainInfo {
        name: "arbitrum-sepolia",
        aliases: &["arb-sepolia", "421614"],
        chain_id: 421614,
        currency_symbol: "ETH",
        is_testnet: true,
        explorer_url: "https://sepolia.arbiscan.io",
    },
    ChainInfo {
        name: "optimism-sepolia",
        aliases: &["op-sepolia", "11155420"],
        chain_id: 11155420,
        currency_symbol: "ETH",
        is_testnet: true,
        explorer_url: "https://sepolia-optimism.etherscan.io",
    },
    ChainInfo {
        name: "base-sepolia",
        aliases: &["84532"],
        chain_id: 84532,
        currency_symbol: "ETH",
        is_testnet: true,
        explorer_url: "https://sepolia.basescan.org",
    },
    ChainInfo {
        name: "linea-sepolia",
        aliases: &["59141"],
        chain_id: 59141,
        currency_symbol: "ETH",
        is_testnet: true,
        explorer_url: "https://sepolia.lineascan.build",
    },
    ChainInfo {
        name: "scroll-sepolia",
        aliases: &["534351"],
        chain_id: 534351,
        currency_symbol: "ETH",
        is_testnet: true,
        explorer_url: "https://sepolia.scrollscan.com",
    },
    ChainInfo {
        name: "blast-sepolia",
        aliases: &["168587773"],
        chain_id: 168587773,
        currency_symbol: "ETH",
        is_testnet: true,
        explorer_url: "https://sepolia.blastscan.io",
    },
];

pub fn resolve_chain(input: &str) -> Result<&'static ChainInfo> {
    let lower = input.to_lowercase();
    for chain in CHAINS {
        if chain.name == lower {
            return Ok(chain);
        }
        if chain.aliases.contains(&lower.as_str()) {
            return Ok(chain);
        }
        // numeric chain_id match
        if let Ok(id) = input.parse::<u64>() {
            if chain.chain_id == id {
                return Ok(chain);
            }
        }
    }
    Err(ScanevmError::UnknownChain(input.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_by_name() {
        assert_eq!(resolve_chain("ethereum").unwrap().chain_id, 1);
        assert_eq!(resolve_chain("polygon").unwrap().chain_id, 137);
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
    }

    #[test]
    fn resolve_unknown() {
        assert!(resolve_chain("notachain").is_err());
    }
}
