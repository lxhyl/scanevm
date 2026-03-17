# etherscan-cli

A fast, multi-chain blockchain explorer CLI built in Rust. Query balances, transactions, contracts, gas prices, and more — directly from your terminal.

## Features

- **Multi-chain** — Ethereum, Polygon, BSC, Arbitrum, Optimism, Base, Avalanche, Linea, Scroll, and more
- **10 commands** — balance, transactions, token transfers, contract source/ABI, gas prices, token info, tx details, block details
- **Flexible output** — human-readable tables or `--json` for scripting
- **Zero runtime deps** — single static binary, no Node/Python required

## Installation

### Homebrew (macOS/Linux)

```sh
brew install lxhyl/tap/etherscan-cli
```

### cargo

```sh
cargo install etherscan-cli
```

### Pre-built binary

Download from [Releases](https://github.com/lxhyl/etherscan-cli/releases), then put the binary somewhere on your `$PATH`.

### Auto-update script

Keep your local install up to date automatically:

```sh
curl -fsSL https://raw.githubusercontent.com/lxhyl/etherscan-cli/main/install.sh | sh
```

## Setup

Get a free API key from [etherscan.io](https://etherscan.io/apis) (works for most chains).

```sh
etherscan config set-key <YOUR_API_KEY>
```

## Commands

```
etherscan balance <ADDRESS> -c <CHAIN>
etherscan txlist <ADDRESS> -c <CHAIN>
etherscan transfers <ADDRESS> -c <CHAIN>
etherscan contract <ADDRESS> -c <CHAIN>
etherscan gas -c <CHAIN>
etherscan token <CONTRACT_ADDRESS> -c <CHAIN>
etherscan tx <TX_HASH> -c <CHAIN>
etherscan block <BLOCK_NUMBER> -c <CHAIN>
etherscan chains
etherscan config
```

## Examples

```sh
# ETH balance
etherscan balance 0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045 -c ethereum

# Recent transactions
etherscan txlist 0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045 -c ethereum

# Gas prices on Base
etherscan gas -c base

# Contract source code
etherscan contract 0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48 -c ethereum

# JSON output for scripting
etherscan balance 0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045 -c ethereum --json | jq .balance
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
