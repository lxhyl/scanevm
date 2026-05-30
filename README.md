# scanevm

A fast CLI for fetching verified smart contract source code — and querying blockchain data — across multiple chains.

```sh
# Fetch USDC source code and save all files locally
scanevm contract source 0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48 -c ethereum --save ./usdc
```

## Contract Commands

This is the primary use case. Every subcommand works on any supported chain.

### `contract source` — download verified source code

```sh
# Show contract metadata (name, compiler, EVM version, license, proxy status)
scanevm contract source <ADDRESS> -c <CHAIN>

# Print source to stdout
scanevm contract source <ADDRESS> -c <CHAIN> --print

# Save all source files to a directory (original layout preserved)
scanevm contract source <ADDRESS> -c <CHAIN> --save ./output

# Flatten a multi-file contract into a single stream / file
scanevm contract source <ADDRESS> -c <CHAIN> --flatten
scanevm contract source <ADDRESS> -c <CHAIN> --flatten --save ./output

# Proxy? Fetch the IMPLEMENTATION's source in one step
scanevm contract source <PROXY> -c <CHAIN> --impl --save ./impl
```

Multi-file contracts (Hardhat / standard JSON input format) are automatically unpacked — the original directory structure is preserved under `./output`.

The metadata view shows proxy status and the implementation address:

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
scanevm contract abi <ADDRESS> -c <CHAIN>

# Compact JSON for piping to jq
scanevm contract abi <ADDRESS> -c <CHAIN> --json

# For a proxy, fetch the implementation's ABI (the one you can actually call)
scanevm contract abi <PROXY> -c <CHAIN> --impl
```

### `contract impl` — resolve a proxy's implementation address

Works even when Etherscan hasn't flagged the contract as a proxy — it reads the
EIP-1967, EIP-1822 (UUPS), beacon, and legacy storage slots directly. If the
address is a multi-facet diamond, it tells you to use `contract facets` instead
of returning a single misleading address.

```sh
scanevm contract impl <PROXY> -c <CHAIN>
```

### `contract facets` — list the facets of a diamond / multi-facet proxy

Resolves the full facet set four ways: DiamondLoupe `facets()` / `facetAddresses()`,
standard EIP-2535 `DiamondCut` event replay, and `SelectorToFacetSet` event replay
for Pendle-style custom selector routers (which expose neither the loupe nor
`DiamondCut`).

```sh
scanevm contract facets <DIAMOND> -c <CHAIN>

# Download every facet's source (one subdir per facet) — the "get all the
# implementation code" command for diamonds, since they have no single impl
scanevm contract facets <DIAMOND> -c <CHAIN> --save ./facets
scanevm contract facets <DIAMOND> -c <CHAIN> --save ./facets --flatten

# Merge every facet's ABI into one combined ABI (the diamond's full interface)
scanevm contract facets <DIAMOND> -c <CHAIN> --abi
```

For a normal (single-implementation) proxy, use `contract source <ADDR> --impl`
instead. If you run `--impl` on a diamond it tells you to use `contract facets`.

### `contract bytecode` — fetch deployed bytecode

```sh
scanevm contract bytecode <ADDRESS> -c <CHAIN>
```

## Other Commands

```
scanevm balance <ADDRESS> -c <CHAIN>       ETH balance
scanevm txlist  <ADDRESS> -c <CHAIN>       Recent transactions
scanevm transfers <ADDRESS> -c <CHAIN>     ERC-20 / NFT transfers
scanevm gas -c <CHAIN>                     Current gas prices
scanevm token <CONTRACT> -c <CHAIN>        ERC-20 token info
scanevm tx <TX_HASH> -c <CHAIN>            Transaction details
scanevm block <NUMBER> -c <CHAIN>          Block details
scanevm chains                             List supported chains
```

Add `--json` to any command for machine-readable output.

## Scripting & agents

Add `--json` for machine-readable output (kept clean on stdout — progress notes
go to stderr). Errors exit with a distinct code per failure class, so scripts and
agents can branch — e.g. retry on `4` but give up on `2`:

| Code | Meaning |
|------|---------|
| 0 | success |
| 2 | usage / config error (missing key, unknown chain, bad input) |
| 3 | network error |
| 4 | rate limited (retryable) |
| 5 | invalid API key |
| 6 | API error |
| 7 | not found (no such block / tx) |
| 8 | contract source not verified |

Environment variables: `ETHERSCAN_API_KEY` (overrides the config file),
`SCANEVM_NO_CACHE=1` (disable the local response cache). Config lives at
`~/.scanevm/config.json` (written `0600`); cache at `~/.scanevm/cache/`.

## Installation

### cargo

```sh
cargo install scanevm
```

### Pre-built binary

Download from [Releases](https://github.com/lxhyl/scanevm/releases/latest) and put it on your `$PATH`.

### Auto-install / update script

```sh
curl -fsSL https://raw.githubusercontent.com/lxhyl/scanevm/main/install.sh | sh
```

Re-run anytime to update to the latest version.

## Setup

Get a free API key at [etherscan.io/apis](https://etherscan.io/apis) — the same key works for most EVM-compatible chains.

```sh
scanevm config set-key <YOUR_API_KEY>
```

You can also set the `ETHERSCAN_API_KEY` environment variable, which overrides the config file.

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

Run `scanevm chains` for the full list including testnets.

## License

MIT
