# scanevm

A fast CLI for fetching verified smart contract source code, ABIs and bytecode across
EVM chains, from Sourcify, Etherscan, Blockscout and Routescan. **No API key needed.**

```sh
# Fetch USDC's implementation source and save all files locally
scanevm contract source 0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48 -c ethereum --impl --save ./usdc
```

## Where the source comes from

Each chain lists its providers in order (`scanevm chains` shows them). scanevm asks them
in that order and keeps the best match:

| Rank | Provider | Key |
|------|----------|-----|
| 1 | Sourcify, exact match | none |
| 2 | Etherscan v2 | your own (optional) |
| 3 | Sourcify, partial match | none |
| 4 | Blockscout (where the chain has an instance) | none |
| 5 | Routescan | none |

It stops as soon as no later provider could do better. A provider that fails (rate limit,
network) is noted and the next one is asked. Without an Etherscan key, Etherscan is
skipped and everything else works; `--json` reports what each provider answered.

Chain state (bytecode, proxy storage slots, loupe calls) is read over the chain's public
RPCs, with Etherscan as the fallback when a key is configured.

## Contract Commands

### `contract source` — download verified source code

```sh
# Show contract metadata (name, compiler, EVM version, license, proxy hint, provider)
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

Multi-file contracts are unpacked with their original directory structure under
`./output`. Source paths come from the provider, so unsafe ones are mapped to safe ones
(`..` becomes `__`, absolute paths become relative).

```
Contract     FiatTokenProxy
Compiler     0.4.24+commit.e67f0147
EVM Version  byzantium
Optimization No
Proxy        Yes (impl: 0x43506849D7C04F9138D1A2050bbF3A0c054402dd)
Source       sourcify (partial)
Files        1
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

### `contract impl` — what runs behind a proxy

Reads the chain directly, so it works whether or not an explorer flagged the contract.
Detection order (the same as the scan core's):

| Kind | How it's found |
|------|----------------|
| `eip7702` | EOA code `0xef0100…` (delegation designator) |
| `eip1167` | minimal-proxy bytecode, vanity variants too |
| `eip1967` | implementation slot; `transparent` (admin slot set) or `uups` (`proxiableUUID()`) |
| `eip1967-beacon` | beacon slot, then `implementation()` / `childImplementation()` on the beacon |
| `eip1822` | UUPS `PROXIABLE` slot |
| `zeppelinos` | legacy ZeppelinOS slot |
| `safe` | Safe proxy: `masterCopy` at slot 0 |
| `diamond` | EIP-2535 loupe `facets()` / `facetAddresses()`, or `DiamondCut` replay |
| `selector-router` | Pendle-style routers: `SelectorToFacetSet` replay |
| `implementation-call` | `implementation()` on the contract (low confidence) |
| `provider-hint` | a provider's proxy flag, when the chain can't confirm (low confidence) |

```sh
scanevm contract impl <PROXY> -c <CHAIN>
```

For a diamond it tells you to use `contract facets` instead of returning a single
misleading address. Event replay needs full-history logs, which public RPCs usually
refuse: with an Etherscan key it uses Etherscan's logs API.

### `contract facets` — list the facets of a diamond / multi-facet proxy

```sh
scanevm contract facets <DIAMOND> -c <CHAIN>

# Download every facet's source (one subdir per facet) — the "get all the
# implementation code" command for diamonds, since they have no single impl
scanevm contract facets <DIAMOND> -c <CHAIN> --save ./facets
scanevm contract facets <DIAMOND> -c <CHAIN> --save ./facets --flatten

# Merge every facet's ABI into one combined ABI (the diamond's full interface)
scanevm contract facets <DIAMOND> -c <CHAIN> --abi
```

Facets are fetched four at a time. For a normal (single-implementation) proxy, use
`contract source <ADDR> --impl` instead.

### `contract bytecode` — fetch deployed bytecode

```sh
scanevm contract bytecode <ADDRESS> -c <CHAIN>
```

## Other Commands

```
scanevm chains                             List supported chains and their providers
scanevm config ...                         Optional Etherscan key(s) and defaults
```

## Scripting & agents

Add `--json` for machine-readable output (kept clean on stdout — progress notes go to
stderr). The shapes are the scan service's, so tools can read either:

| Command | `--json` prints |
|---------|-----------------|
| `contract source` | `{target, chain_id, address, status, code, source: {meta}, attempts, error}` — `meta` has `name`, `language`, `compiler`, `provider`, `match`, `providers_tried`, `proxy_hint`, `deployment`, `license`, `files` |
| `contract impl` | `{target, chain_id, address, status, kind, variant, confidence, implementation, beacon, admin, facets, checked_at}` |
| `contract facets` | `{root, resolution, implementations}` — one source report per facet |
| `contract bytecode` | `{chain_id, address, codehash, size, delegation, code}` |
| `contract abi` | the ABI array |
| `chains` | `{chains: [{id, name, title, aliases, testnet, explorer, sources}]}` |

`status` is `cached`, `fetched`, `not_verified`, `no_code` or `error`; the source report
is printed for unverified contracts too. Errors exit with a distinct code per failure
class, so scripts and agents can branch — e.g. retry on `4` but give up on `2`:

| Code | Meaning |
|------|---------|
| 0 | success |
| 2 | usage / config error (unknown chain, bad input) |
| 3 | network error |
| 4 | rate limited (retryable) |
| 5 | invalid API key |
| 6 | API error (also: not a proxy / not a diamond) |
| 7 | not found (no contract code at the address) |
| 8 | contract source not verified |

Environment variables: `ETHERSCAN_API_KEYS` / `ETHERSCAN_API_KEY` (a key or
comma-separated pool; overrides the config file), `SCANEVM_NO_CACHE=1` (disable the
local cache). Config lives at `~/.scanevm/config.json` (written `0600`); cache at
`~/.scanevm/cache/`.

## Caching

Results are cached on disk at `~/.scanevm/cache/`, with the lifetime chosen by how
mutable the data is:

| Data | Cached |
|------|--------|
| Verified **non-proxy** source and ABI (keyed by the code's hash), bytecode | permanently (immutable) |
| Source of a contract a provider flags as a **proxy** | **never — always fetched fresh** |
| Proxy storage slots | 60s |
| Unverified results | not cached |

To force a fully live run, pass `--no-cache` (any command) or set `SCANEVM_NO_CACHE=1`:

```sh
scanevm contract source 0x... -c eth --no-cache   # skip cache read + write
```

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

## Etherscan key (optional)

scanevm works without a key. An Etherscan key adds Etherscan as a source, a fallback for
chain reads, and its logs API for diamond event replay. Get a free one at
[etherscan.io/apis](https://etherscan.io/apis) — the same key works for every chain
Etherscan v2 serves.

```sh
scanevm config set-key <YOUR_API_KEY>
```

You can also set the `ETHERSCAN_API_KEY` environment variable, which overrides the config file.

### API key pool (higher throughput)

Etherscan rate-limits each key to **5 requests/second** on the free tier. Configure
**several keys** and scanevm pools them — each request picks the key that's been idle
longest and each key is throttled independently, so *N* keys sustain roughly *N×* the
rate limit. If one key gets rate-limited mid-run, scanevm cools it down and rotates to
another key automatically.

```sh
# Set the whole pool at once (space- or comma-separated)
scanevm config set-key KEY_1 KEY_2 KEY_3

# Or build it up / trim it
scanevm config add-key KEY_4
scanevm config remove-key KEY_2

# See how many keys are configured (keys are shown masked)
scanevm config show
```

Via environment (overrides the config file), use `ETHERSCAN_API_KEYS` for a list:

```sh
export ETHERSCAN_API_KEYS="KEY_1,KEY_2,KEY_3"
```

A single `ETHERSCAN_API_KEY` still works and also accepts a comma-separated list.

## Supported Chains

23 mainnets and 8 testnets, each with its own provider order (run `scanevm chains`
for the full table with sources and explorers):

| Name | Aliases | Chain ID |
|------|---------|----------|
| ethereum | eth, mainnet | 1 |
| polygon | matic, pol | 137 |
| bsc | bnb, binance | 56 |
| arbitrum | arb, arb1, arbitrum-one | 42161 |
| optimism | op | 10 |
| base | — | 8453 |
| avalanche | avax, avalanche-c | 43114 |
| linea | — | 59144 |
| scroll | — | 534352 |
| zksync | zks, zksync-era | 324 |
| hyperevm | hyper, hyperliquid, hl | 999 |
| sonic | — | 146 |
| mantle | — | 5000 |
| berachain | bera | 80094 |
| monad | — | 143 |

Also: blast, celo, gnosis, fantom, cronos, moonriver, xlayer, plasma, and the testnets
sepolia, amoy, bsc-testnet, arbitrum-sepolia, optimism-sepolia, base-sepolia,
linea-sepolia, blast-sepolia. Any other numeric chain id (or `0x` hex id) works too.

## License

MIT
