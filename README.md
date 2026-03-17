# etherscan-cli

A fast CLI for fetching verified smart contract source code — and querying blockchain data — across multiple chains.

```sh
# Fetch USDC source code and save all files locally
etherscan contract source 0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48 -c ethereum --save ./usdc
```

## Contract Commands

This is the primary use case. All three subcommands work on any supported chain.

### `contract source` — download verified source code

```sh
# Show contract metadata (name, compiler, license, proxy status)
etherscan contract source <ADDRESS> -c <CHAIN>

# Print source to stdout
etherscan contract source <ADDRESS> -c <CHAIN> --print

# Save all source files to a directory
etherscan contract source <ADDRESS> -c <CHAIN> --save ./output
```

Multi-file contracts (Hardhat / standard JSON input format) are automatically unpacked — the original directory structure is preserved under `./output`.

Proxy contracts show the implementation address:

```
Contract     TransparentUpgradeableProxy
Compiler     v0.8.4+commit.c7e474f2
Optimization Yes (200 runs)
License      MIT
Proxy        Yes (impl: 0x43506849D7C04F9138D1A2050bbF3A0c054402dd)
Chain        ethereum
```

### `contract abi` — fetch ABI as pretty-printed JSON

```sh
etherscan contract abi <ADDRESS> -c <CHAIN>
```

### `contract bytecode` — fetch deployed bytecode

```sh
etherscan contract bytecode <ADDRESS> -c <CHAIN>
```

## Other Commands

```
etherscan balance <ADDRESS> -c <CHAIN>       ETH balance
etherscan txlist  <ADDRESS> -c <CHAIN>       Recent transactions
etherscan transfers <ADDRESS> -c <CHAIN>     ERC-20 / NFT transfers
etherscan gas -c <CHAIN>                     Current gas prices
etherscan token <CONTRACT> -c <CHAIN>        ERC-20 token info
etherscan tx <TX_HASH> -c <CHAIN>            Transaction details
etherscan block <NUMBER> -c <CHAIN>          Block details
etherscan chains                             List supported chains
```

Add `--json` to any command for machine-readable output.

## Installation

### Pre-built binary

Download from [Releases](https://github.com/lxhyl/etherscan-cli/releases/latest) and put it on your `$PATH`.

### Auto-install / update script

```sh
curl -fsSL https://raw.githubusercontent.com/lxhyl/etherscan-cli/main/install.sh | sh
```

Re-run anytime to update to the latest version.

### cargo

```sh
cargo install etherscan-cli
```

## Setup

Get a free API key at [etherscan.io/apis](https://etherscan.io/apis) — the same key works for most EVM-compatible chains.

```sh
etherscan config set-key <YOUR_API_KEY>
```

## Supported Chains

| Name | Aliases | Chain ID |
|------|---------|----------|
| ethereum | eth, mainnet | 1 |
| polygon | matic | 137 |
| bsc | bnb, binance | 56 |
| arbitrum | arb | 42161 |
| optimism | op | 10 |
| base | — | 8453 |
| avalanche | avax | 43114 |
| linea | — | 59144 |
| scroll | — | 534352 |

Run `etherscan chains` for the full list including testnets.

## License

MIT
